// The web page's host (nsh.json `web.host`): what the app asks of the
// browser it runs in, where NSH has no module of its own. After noop's
// app/host.js (MIT).
// - Dictation: the browser's speech recognition (SpeechRecognition,
//   webkitSpeechRecognition), on a secure page only (HTTPS or localhost),
//   where the browser lets a page have the microphone.
// Events to the app: `speech` { text }, the whole transcript so far (final
// and still-being-heard words), and `speechEnd` { error? } once it stops.

const Recognition = () =>
  typeof window === "undefined" || !window.isSecureContext
    ? null
    : window.SpeechRecognition || window.webkitSpeechRecognition || null;

let recognition = null;

function speechStart(player, lang) {
  const R = Recognition();
  if (!R) throw new Error("unavailable");
  if (recognition) recognition.abort();
  const r = new R();
  recognition = r;
  r.lang = lang || navigator.language || "en-US";
  r.continuous = true;
  r.interimResults = true;
  let error = null;
  r.onresult = (e) => {
    let text = "";
    for (let i = 0; i < e.results.length; i++) text += e.results[i][0].transcript;
    player.hostEvent("speech", { text });
  };
  r.onerror = (e) => (error = e.error || "failed");
  r.onend = () => {
    if (recognition === r) recognition = null;
    player.hostEvent("speechEnd", error && error !== "aborted" ? { error } : {});
  };
  r.start();
}

export function createHost(player) {
  return {
    async call(name, args = {}) {
      switch (name) {
        case "speechState":
          return { supported: !!Recognition() };
        case "speechStart":
          // From the microphone button's press: the browser asks for the
          // microphone only in a user's gesture.
          speechStart(player, args?.lang);
          return {};
        case "speechStop":
          // The words still being heard come as a last `speech`, then `speechEnd`.
          recognition?.stop();
          return {};
        case "speechAbort":
          recognition?.abort();
          return {};
        default:
          throw new Error(`unknown host function ${name}`);
      }
    },
  };
}
