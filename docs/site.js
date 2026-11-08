(() => {
  "use strict";
  const root = document.documentElement;
  const systemDark = window.matchMedia("(prefers-color-scheme: dark)");
  const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)");
  const readTheme = () => {
    try { const value = localStorage.getItem("quiverdl-theme"); return value === "light" || value === "dark" ? value : null; }
    catch { return null; }
  };
  const applyTheme = (theme) => {
    root.dataset.theme = theme;
    document.querySelectorAll("[data-theme-toggle]").forEach((button) => {
      button.setAttribute("aria-pressed", String(theme === "dark"));
      button.setAttribute("aria-label", theme === "dark" ? "Switch to light theme" : "Switch to dark theme");
    });
    const meta = document.querySelector('meta[name="theme-color"]:not([media])');
    if (meta) meta.content = theme === "dark" ? "#101b21" : "#f5f7f8";
  };
  applyTheme(readTheme() || (systemDark.matches ? "dark" : "light"));
  document.querySelectorAll("[data-theme-toggle]").forEach((button) => button.addEventListener("click", () => {
    const next = root.dataset.theme === "dark" ? "light" : "dark";
    applyTheme(next);
    try { localStorage.setItem("quiverdl-theme", next); } catch { /* Theme remains usable without storage. */ }
  }));
  systemDark.addEventListener("change", () => {
    if (!readTheme()) applyTheme(systemDark.matches ? "dark" : "light");
  });

  const nav = document.querySelector("[data-nav]");
  const navToggle = document.querySelector("[data-nav-toggle]");
  const closeNav = () => {
    nav?.classList.remove("open");
    navToggle?.setAttribute("aria-expanded", "false");
    navToggle?.setAttribute("aria-label", "Open navigation");
  };
  navToggle?.addEventListener("click", () => {
    const open = navToggle.getAttribute("aria-expanded") !== "true";
    nav?.classList.toggle("open", open);
    navToggle.setAttribute("aria-expanded", String(open));
    navToggle.setAttribute("aria-label", open ? "Close navigation" : "Open navigation");
  });
  nav?.querySelectorAll("a").forEach((link) => link.addEventListener("click", closeNav));
  document.addEventListener("keydown", (event) => {
    if (event.key === "Escape" && navToggle?.getAttribute("aria-expanded") === "true") {
      closeNav();
      navToggle.focus();
    }
  });
  document.addEventListener("click", (event) => {
    if (nav && navToggle && !nav.contains(event.target) && !navToggle.contains(event.target)) closeNav();
  });
  const header = document.querySelector("[data-header]");
  const updateHeader = () => header?.classList.toggle("scrolled", window.scrollY > 12);
  window.addEventListener("scroll", updateHeader, { passive: true });
  updateHeader();

  const tabs = Array.from(document.querySelectorAll("[data-preview-tab]"));
  const selectTab = (selected) => {
    tabs.forEach((tab) => {
      const active = tab === selected;
      tab.setAttribute("aria-selected", String(active));
      tab.tabIndex = active ? 0 : -1;
    });
    document.querySelectorAll("[data-preview-panel]").forEach((panel) => {
      panel.hidden = panel.dataset.previewPanel !== selected.dataset.previewTab;
    });
  };
  tabs.forEach((tab, index) => {
    tab.addEventListener("click", () => selectTab(tab));
    tab.addEventListener("keydown", (event) => {
      let next;
      if (event.key === "ArrowRight") next = (index + 1) % tabs.length;
      if (event.key === "ArrowLeft") next = (index + tabs.length - 1) % tabs.length;
      if (event.key === "Home") next = 0;
      if (event.key === "End") next = tabs.length - 1;
      if (next !== undefined) {
        event.preventDefault();
        selectTab(tabs[next]);
        tabs[next].focus();
      }
    });
  });

  const demo = document.querySelector("[data-demo]");
  if (!demo) return;
  const pause = document.querySelector("[data-demo-toggle]");
  const replay = document.querySelector("[data-demo-replay]");
  const total = 256;
  const speed = 8;
  let downloaded = 64;
  let paused = reducedMotion.matches;
  let last = performance.now();
  const update = () => {
    const complete = downloaded >= total;
    const state = complete ? "Complete" : paused ? "Paused" : "Downloading";
    document.querySelectorAll("[data-progress-bar]").forEach((bar) => { bar.style.width = (downloaded / total * 100) + "%"; });
    document.querySelectorAll("[data-progress-label]").forEach((label) => { label.textContent = Math.floor(downloaded / total * 100) + "%"; });
    document.querySelectorAll("[data-demo-downloaded]").forEach((label) => { label.textContent = downloaded.toFixed(1) + " MB"; });
    document.querySelectorAll("[data-speed]").forEach((label) => { label.textContent = paused || complete ? "—" : speed.toFixed(1) + " MiB/s"; });
    document.querySelectorAll("[data-demo-eta]").forEach((label) => { label.textContent = complete ? "Complete" : paused ? "Paused" : Math.ceil((total - downloaded) / speed) + "s"; });
    document.querySelectorAll("[data-demo-status]").forEach((label) => { label.textContent = state; });
    const queueStatus = document.querySelector(".queue-row small");
    const queueState = document.querySelector(".queue-state");
    if (queueStatus) queueStatus.textContent = complete ? "Download complete" : paused ? "Paused by you" : "Downloading now";
    if (queueState) queueState.textContent = complete ? "Complete" : paused ? "Paused" : "Active";
    if (pause) {
      pause.textContent = complete ? "Preview complete" : paused ? "Resume preview" : "Pause preview";
      pause.setAttribute("aria-pressed", String(paused));
      pause.disabled = complete;
    }
  };
  pause?.addEventListener("click", () => { paused = !paused; last = performance.now(); update(); });
  replay?.addEventListener("click", () => { downloaded = 64; paused = reducedMotion.matches; last = performance.now(); update(); });
  reducedMotion.addEventListener("change", () => {
    if (reducedMotion.matches) { paused = true; update(); }
  });
  update();
  window.setInterval(() => {
    const now = performance.now();
    const elapsed = Math.min(2, (now - last) / 1000);
    last = now;
    if (paused || document.hidden || downloaded >= total) return;
    downloaded = Math.min(total, downloaded + speed * elapsed);
    update();
  }, 750);
})();

