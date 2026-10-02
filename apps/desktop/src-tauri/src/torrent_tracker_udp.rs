use super::{MAX_INITIAL_PEERS, is_public_ip};
use quiver_core::DownloadControl;
use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    time::Duration,
};
use tokio::net::UdpSocket;
use url::Url;

pub(super) async fn announce(
    tracker: &Url,
    addresses: &[SocketAddr],
    hash: &[u8; 20],
    peer: &[u8; 20],
    control: &DownloadControl,
) -> Result<Vec<SocketAddr>, String> {
    for address in addresses.iter().take(4) {
        let result = tokio::select! {
            result = tokio::time::timeout(Duration::from_secs(35), exchange(tracker, *address, hash, peer)) => result,
            _ = control.cancelled() => return Err("download was cancelled".into()),
        };
        if let Ok(Ok(peers)) = result
            && !peers.is_empty()
        {
            return Ok(peers);
        }
    }
    Err("UDP tracker returned no usable peers".into())
}

async fn exchange(
    tracker: &Url,
    address: SocketAddr,
    hash: &[u8; 20],
    peer: &[u8; 20],
) -> Result<Vec<SocketAddr>, String> {
    // Callers supply only addresses already classified and pinned by the resolver.
    let socket = UdpSocket::bind(if address.is_ipv4() {
        "0.0.0.0:0"
    } else {
        "[::]:0"
    })
    .await
    .map_err(|_| "UDP socket unavailable")?;
    socket
        .connect(address)
        .await
        .map_err(|_| "UDP tracker connection failed")?;
    let transaction = rand::random::<u32>();
    let mut connect = Vec::from(0x41727101980_u64.to_be_bytes());
    connect.extend_from_slice(&0_u32.to_be_bytes());
    connect.extend_from_slice(&transaction.to_be_bytes());
    let reply = request(&socket, &connect, 0, transaction).await?;
    let connection: [u8; 8] = reply
        .get(8..16)
        .ok_or("Truncated UDP connect response")?
        .try_into()
        .unwrap();
    let transaction = rand::random::<u32>();
    let packet = announce_packet(tracker, connection, transaction, hash, peer)?;
    let reply = request(&socket, &packet, 1, transaction).await?;
    parse_peers(&reply, address.is_ipv6())
}

async fn request(
    socket: &UdpSocket,
    packet: &[u8],
    action: u32,
    transaction: u32,
) -> Result<Vec<u8>, String> {
    let mut buffer = vec![0; 65536];
    // BEP 15 starts retransmission after 15 seconds. The enclosing exchange is bounded.
    for seconds in [15, 30] {
        socket
            .send(packet)
            .await
            .map_err(|_| "UDP tracker send failed")?;
        let receive = async {
            loop {
                let size = socket
                    .recv(&mut buffer)
                    .await
                    .map_err(|_| "UDP tracker receive failed")?;
                if size < 8 || buffer[4..8] != transaction.to_be_bytes() {
                    continue;
                }
                if buffer[..4] != action.to_be_bytes() {
                    return Err("UDP tracker rejected the request");
                }
                return Ok(buffer[..size].to_vec());
            }
        };
        if let Ok(result) = tokio::time::timeout(Duration::from_secs(seconds), receive).await {
            return result.map_err(str::to_owned);
        }
    }
    Err("UDP tracker timed out".into())
}

fn announce_packet(
    tracker: &Url,
    connection: [u8; 8],
    transaction: u32,
    hash: &[u8; 20],
    peer: &[u8; 20],
) -> Result<Vec<u8>, String> {
    let mut packet = Vec::from(connection);
    packet.extend_from_slice(&1_u32.to_be_bytes());
    packet.extend_from_slice(&transaction.to_be_bytes());
    packet.extend_from_slice(hash);
    packet.extend_from_slice(peer);
    packet.extend_from_slice(&0_u64.to_be_bytes()); // downloaded
    packet.extend_from_slice(&1_u64.to_be_bytes()); // left: discovery, not a completed seed
    packet.extend_from_slice(&0_u64.to_be_bytes()); // uploaded
    packet.extend_from_slice(&0_u32.to_be_bytes()); // event
    packet.extend_from_slice(&0_u32.to_be_bytes()); // source IP
    packet.extend_from_slice(&rand::random::<u32>().to_be_bytes());
    packet.extend_from_slice(&(MAX_INITIAL_PEERS as u32).to_be_bytes());
    packet.extend_from_slice(&0_u16.to_be_bytes()); // no inbound listener
    // BEP 41 carries the original encoded path/query, including private passkeys.
    let mut target = tracker.path().to_owned();
    if let Some(query) = tracker.query() {
        target.push('?');
        target.push_str(query);
    }
    if target.len() > 4096 {
        return Err("UDP tracker path exceeds the safety limit".into());
    }
    for chunk in target.as_bytes().chunks(255) {
        packet.extend_from_slice(&[2, chunk.len() as u8]);
        packet.extend_from_slice(chunk);
    }
    Ok(packet)
}

fn parse_peers(reply: &[u8], ipv6: bool) -> Result<Vec<SocketAddr>, String> {
    let body = reply.get(20..).ok_or("Truncated UDP announce response")?;
    let width = if ipv6 { 18 } else { 6 };
    if body.len() % width != 0 {
        return Err("Malformed UDP peer response".into());
    }
    let mut peers = Vec::new();
    for entry in body.chunks_exact(width) {
        let ip = if ipv6 {
            IpAddr::V6(Ipv6Addr::from(<[u8; 16]>::try_from(&entry[..16]).unwrap()))
        } else {
            IpAddr::V4(Ipv4Addr::new(entry[0], entry[1], entry[2], entry[3]))
        };
        let port = u16::from_be_bytes([entry[width - 2], entry[width - 1]]);
        if port != 0 && is_public_ip(ip) {
            peers.push(SocketAddr::new(ip, port));
        }
        if peers.len() == MAX_INITIAL_PEERS {
            break;
        }
    }
    Ok(peers)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn udp_handshake_checks_transactions_and_preserves_passkey() {
        let server = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let address = server.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let mut buf = [0; 8192];
            let (size, client) = server.recv_from(&mut buf).await.unwrap();
            assert_eq!(size, 16);
            assert_eq!(&buf[..8], &0x41727101980_u64.to_be_bytes());
            let mut response = buf[8..16].to_vec();
            response.extend_from_slice(&42_u64.to_be_bytes());
            let mut wrong = response.clone();
            wrong[4] ^= 1;
            server.send_to(&wrong, client).await.unwrap();
            server.send_to(&response, client).await.unwrap();
            let (size, client) = server.recv_from(&mut buf).await.unwrap();
            assert_eq!(&buf[..8], &42_u64.to_be_bytes());
            assert_eq!(&buf[16..36], &[1; 20]);
            assert_eq!(&buf[98..size], b"\x02\x1b/announce?passkey=synthetic");
            let mut response = buf[8..16].to_vec();
            response.extend_from_slice(&[0; 12]);
            response.extend_from_slice(&[1, 1, 1, 1, 0x1a, 0xe1]);
            server.send_to(&response, client).await.unwrap();
        });
        let tracker = Url::parse("udp://tracker.example:80/announce?passkey=synthetic").unwrap();
        let peers = tokio::time::timeout(
            Duration::from_secs(3),
            exchange(&tracker, address, &[1; 20], &[2; 20]),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(peers, ["1.1.1.1:6881".parse().unwrap()]);
        task.await.unwrap();
    }

    #[test]
    fn rejects_truncation_and_filters_private_peers() {
        assert!(parse_peers(&[0; 19], false).is_err());
        assert!(parse_peers(&[0; 21], false).is_err());
        let mut reply = vec![0; 20];
        reply.extend_from_slice(&[127, 0, 0, 1, 1, 1]);
        reply.extend_from_slice(&[8, 8, 8, 8, 1, 1]);
        assert_eq!(parse_peers(&reply, false).unwrap().len(), 1);
    }

    #[test]
    fn ipv6_peers_and_chunked_url_data_are_preserved() {
        let mut reply = vec![0; 20];
        reply.extend_from_slice(&"2606:4700:4700::1111".parse::<Ipv6Addr>().unwrap().octets());
        reply.extend_from_slice(&6881_u16.to_be_bytes());
        assert_eq!(
            parse_peers(&reply, true).unwrap(),
            ["[2606:4700:4700::1111]:6881".parse().unwrap()]
        );
        let target = format!("/announce?passkey={}", "a".repeat(300));
        let tracker = Url::parse(&format!("udp://tracker.example:80{target}")).unwrap();
        let packet = announce_packet(&tracker, [0; 8], 1, &[0; 20], &[0; 20]).unwrap();
        let mut rest = &packet[98..];
        let mut restored = Vec::new();
        while !rest.is_empty() {
            assert_eq!(rest[0], 2);
            let length = rest[1] as usize;
            restored.extend_from_slice(&rest[2..2 + length]);
            rest = &rest[2 + length..];
        }
        assert_eq!(restored, target.as_bytes());
    }
}
