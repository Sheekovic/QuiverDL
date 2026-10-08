import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import vm from "node:vm";
import { test } from "node:test";
const source = await readFile(new URL("firefox/transport.js", import.meta.url), "utf8");
const token = "ab".repeat(32);
const ack = { ok: true, requestId: "12345678-1234-1234-1234-123456789abc" };
function fixture({ stored = {}, sessionError, messageError, responses = [] } = {}) {
  const requests = [];
  let native = 0;
  const context = { AbortSignal, TypeError, TextDecoder, Uint8Array,
    browser: { storage: { local: {
      async get() { return { ...stored }; }, async set(v) { Object.assign(stored,v); },
      async remove(k) { delete stored[k]; },
    } }, runtime: { async sendNativeMessage() { native++; return ack; } } },
    async fetch(url, options) {
      requests.push({url,options});
      if (url.endsWith('/session')) {
        if(sessionError) throw sessionError;
        return Response.json({ protocol:'quiverdl', version:1, token });
      }
      if(messageError) throw messageError;
      return responses.shift() ?? Response.json(ack);
    },
  };
  vm.runInNewContext(source,context);
  return { send:context.quiverTransport.send, requests, stored, native:()=>native };
}
const message = {version:1,action:'enqueue',url:'https://example.test/file'};
test('automatic session and handoff require no stored code',async()=>{
  const f=fixture(); await f.send(message);
  assert.equal(f.native(),0); assert.equal(f.requests.length,2);
  assert.equal(f.stored.quiverStoreTransport,true);
  assert.equal(JSON.stringify(f.stored).includes(token),false);
  const {options}=f.requests[1];
  assert.equal(options.headers.Authorization,`Bearer ${token}`);
  assert.equal(options.headers['X-QuiverDL-Connector'],'1');
  assert.equal(options.redirect,'error'); assert.equal(options.credentials,'omit');
  assert.equal(options.body.includes(token),false);
});
test('missing service preserves native installations on initial discovery',async()=>{
  const f=fixture({sessionError:new TypeError('offline')});await f.send(message);
  assert.equal(f.native(),1);
});
test('previous Store connection and obsolete pairing setting never select native',async()=>{
  for(const stored of [{quiverStoreTransport:true},{storePairingCode:token}]){
    const f=fixture({stored,sessionError:new TypeError('offline')});
    await assert.rejects(f.send(message)); assert.equal(f.native(),0);
    assert.equal(f.stored.storePairingCode,undefined);
  }
});
test('lost enqueue acknowledgement never retries or falls back',async()=>{
  const f=fixture({messageError:new TypeError('lost response')});
  await assert.rejects(f.send(message)); assert.equal(f.requests.length,2);assert.equal(f.native(),0);
});
test('restart or expired session refreshes only after a pre-mutation 401',async()=>{
  const f=fixture({responses:[new Response('',{status:401}),Response.json(ack)]});
  await f.send(message);assert.equal(f.requests.length,4);assert.equal(f.native(),0);
});
test('malformed and oversized acknowledgements fail closed',async()=>{
  for(const response of [Response.json({ok:true}),Response.json({ok:false}), new Response('x'.repeat(4097))]){
    const f=fixture({responses:[response]});await assert.rejects(f.send(message));
    assert.equal(f.native(),0);assert.equal(f.requests.length,2);
  }
});

test('settings requests missing permission directly in the click gesture', async () => {
  const optionsSource = await readFile(new URL("firefox/options.js", import.meta.url), "utf8");
  for (const granted of [true, false]) {
  const nodes = new Map();
  let requested = false;
  const context = {
    document: { querySelector(selector) {
      if (!nodes.has(selector)) nodes.set(selector, { addEventListener(_event, handler) { this.click = handler; } });
      return nodes.get(selector);
    } },
    browser: {
      storage: { local: { async get(defaults) { return defaults; } } },
      permissions: {
        contains() { assert.fail("Permission request must not follow an asynchronous permission check"); },
        request() { requested = true; return Promise.resolve(granted); },
      },
    },
    quiverTransport: { async send() { return { ok: true }; } },
  };
  vm.runInNewContext(optionsSource, context);
  const completed = nodes.get("#check").click();
  assert.equal(requested, true, "Permission request happens synchronously with the click");
  await completed;
  assert.equal(nodes.get("#status").textContent, "Connected to QuiverDL.");
  }
});
