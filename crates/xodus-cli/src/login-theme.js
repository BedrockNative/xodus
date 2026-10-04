(() => {
    const css = __XODUS_LOGIN_CSS__;
    const applyTheme = () => {
        if (!document.documentElement) return;
        let style = document.getElementById("xodus-login-theme");
        if (!style) {
            style = document.createElement("style");
            style.id = "xodus-login-theme";
            (document.head || document.documentElement).appendChild(style);
        }
        if (style.textContent !== css) style.textContent = css;
        if (!document.documentElement.classList.contains("xodus-inline-login")) {
            document.documentElement.classList.add("xodus-inline-login");
        }
    };

    // Theme every document in this login WebView, including verification and
    // recovery redirects. This has no connection to the authentication bridge.
    // Observe from document start: waiting for DOMContentLoaded flashes white.
    window.__xodusLoginThemeObserver?.disconnect();
    const observer = new MutationObserver(applyTheme);
    window.__xodusLoginThemeObserver = observer;
    observer.observe(document, {childList: true, subtree: true});
    applyTheme();
    const ready = () => {
        applyTheme();
        // After parsing, watch only stylesheet/root replacement, not every
        // mutation of an authentication form or animation.
        observer.disconnect();
        observer.observe(document, {childList: true});
        if (document.documentElement) {
            observer.observe(document.documentElement, {childList: true, attributes: true, attributeFilter: ["class"]});
        }
        if (document.head) observer.observe(document.head, {childList: true});
    };
    if (document.readyState === "loading") {
        document.addEventListener("DOMContentLoaded", ready, {once: true});
    } else {
        ready();
    }
    window.addEventListener("pagehide", () => observer.disconnect(), {once: true});
})();
