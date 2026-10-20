// Chat Computer in Chrome's side panel is the website itself (chatcomputer.ai, in the frame on panel.html), so it's
// always the newest. It asks the extension for the one thing a web page can't do itself: a site, in a real Chrome
// tab (any site, without a click first). Only Chat Computer's own page can ask, and only for a web address.
const SITE = "https://chatcomputer.ai";
addEventListener("message", (e) => {
  if (e.origin !== SITE || !e.data || e.data.chatcomputer !== "open_tab") return;
  let url;
  try { url = new URL(String(e.data.url)); } catch (x) { return; }
  if (url.protocol !== "https:" && url.protocol !== "http:") return;
  chrome.tabs.create({ url: url.href });
});
