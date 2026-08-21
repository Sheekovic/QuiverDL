(() => {
  const root = document.documentElement;
  const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)");
  const systemDark = window.matchMedia("(prefers-color-scheme: dark)");

  const readTheme = () => {
    try {
      return window.localStorage.getItem("quiverdl-theme");
    } catch {
      return null;
    }
  };

  const writeTheme = (theme) => {
    try {
      window.localStorage.setItem("quiverdl-theme", theme);
    } catch {
      // The theme still works for this page view when storage is unavailable.
    }
  };

  const applyTheme = (theme) => {
    root.dataset.theme = theme;
    document.querySelectorAll("[data-theme-toggle]").forEach((button) => {
      const dark = theme === "dark";
      button.setAttribute("aria-pressed", String(dark));
      button.setAttribute("aria-label", dark ? "Switch to light theme" : "Switch to dark theme");
    });
  };

  applyTheme(readTheme() || (systemDark.matches ? "dark" : "light"));

  document.querySelectorAll("[data-theme-toggle]").forEach((button) => {
    button.addEventListener("click", () => {
      const nextTheme = root.dataset.theme === "dark" ? "light" : "dark";
      applyTheme(nextTheme);
      writeTheme(nextTheme);
    });
  });

  const navToggle = document.querySelector("[data-nav-toggle]");
  const nav = document.querySelector("[data-nav]");

  const closeNav = () => {
    if (!navToggle || !nav) return;
    navToggle.setAttribute("aria-expanded", "false");
    nav.classList.remove("open");
  };

  if (navToggle && nav) {
    navToggle.addEventListener("click", () => {
      const open = navToggle.getAttribute("aria-expanded") !== "true";
      navToggle.setAttribute("aria-expanded", String(open));
      nav.classList.toggle("open", open);
    });

    nav.querySelectorAll("a").forEach((link) => link.addEventListener("click", closeNav));
    document.addEventListener("keydown", (event) => {
      if (event.key === "Escape") closeNav();
    });
    document.addEventListener("click", (event) => {
      if (!nav.contains(event.target) && !navToggle.contains(event.target)) closeNav();
    });
  }

  const header = document.querySelector("[data-header]");
  const updateHeader = () => header?.classList.toggle("scrolled", window.scrollY > 12);
  updateHeader();
  window.addEventListener("scroll", updateHeader, { passive: true });

  const platformValue = `${navigator.userAgentData?.platform || ""} ${navigator.platform || ""} ${navigator.userAgent || ""}`.toLowerCase();
  let platform = "";
  let platformMessage = "";

  if (platformValue.includes("win")) {
    platform = "windows";
    platformMessage = "Windows detected — Microsoft Store is the recommended download for this device.";
  } else if (platformValue.includes("mac")) {
    platform = "macos";
    platformMessage = "macOS detected — native Mac downloads are coming soon.";
  } else if (platformValue.includes("linux") || platformValue.includes("x11")) {
    platform = "linux";
    platformMessage = "Linux detected — AppImage, Debian, and RPM packages are coming soon.";
  }

  if (platform) {
    const platformCard = document.querySelector(`[data-platform="${platform}"]`);
    const platformNote = document.querySelector("[data-platform-note]");
    const platformMessageNode = document.querySelector("[data-platform-message]");
    platformCard?.classList.add("recommended");
    if (platformNote && platformMessageNode) {
      platformMessageNode.textContent = platformMessage;
      platformNote.hidden = false;
    }
  }

  const revealItems = document.querySelectorAll("[data-reveal]");
  if (revealItems.length && !reducedMotion.matches && "IntersectionObserver" in window) {
    root.classList.add("reveal-ready");
    const observer = new IntersectionObserver(
      (entries) => {
        entries.forEach((entry) => {
          if (!entry.isIntersecting) return;
          entry.target.classList.add("is-visible");
          observer.unobserve(entry.target);
        });
      },
      { rootMargin: "0px 0px -7%", threshold: 0.08 }
    );
    revealItems.forEach((item) => observer.observe(item));
  } else {
    revealItems.forEach((item) => item.classList.add("is-visible"));
  }

  const demo = document.querySelector("[data-demo]");
  const demoToggle = document.querySelector("[data-demo-toggle]");
  const progressBar = document.querySelector("[data-progress-bar]");
  const progressLabel = document.querySelector("[data-progress-label]");
  const speedLabel = document.querySelector("[data-speed]");
  let demoPaused = reducedMotion.matches;
  let progress = 78;

  const updateDemoButton = () => {
    if (!demoToggle) return;
    demoToggle.innerHTML = demoPaused
      ? '<span aria-hidden="true">▶</span> Resume'
      : '<span aria-hidden="true">Ⅱ</span> Pause';
  };

  const updateDemo = () => {
    if (demoPaused || !demo || !progressBar || !progressLabel || !speedLabel) return;
    progress += 1;
    if (progress > 94) progress = 67;
    progressBar.style.width = `${progress}%`;
    progressLabel.textContent = `${progress}%`;
    speedLabel.textContent = `${(38 + ((progress * 1.73) % 13)).toFixed(1)} MB/s`;
  };

  updateDemoButton();
  const demoTimer = window.setInterval(updateDemo, 1250);

  demoToggle?.addEventListener("click", () => {
    demoPaused = !demoPaused;
    updateDemoButton();
  });

  window.addEventListener("pagehide", () => window.clearInterval(demoTimer), { once: true });
})();

