window.addEventListener("error", (e) => {
  const pre = document.createElement("pre");
  pre.style.cssText = "position:fixed;top:0;left:0;right:0;background:#400;color:#fff;font-size:11px;padding:6px;z-index:999;white-space:pre-wrap;";
  pre.textContent = "JS ERROR: " + (e.message || e.error) + "\n" + (e.filename || "") + ":" + (e.lineno || "");
  document.body.appendChild(pre);
});
window.addEventListener("unhandledrejection", (e) => {
  const pre = document.createElement("pre");
  pre.style.cssText = "position:fixed;top:0;left:0;right:0;background:#400;color:#fff;font-size:11px;padding:6px;z-index:999;white-space:pre-wrap;";
  pre.textContent = "UNHANDLED REJECTION: " + e.reason;
  document.body.appendChild(pre);
});
