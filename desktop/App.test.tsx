// The window, headless, in a scratch home (test.sh): pages open, a
// provider is added by hand, a group is made from its models, and an
// agent is connected to it with its switch and the model picker.
import { expect, open, test } from "nsh/test";

/** The window at its launch size, on a desktop. */
const launch = () => open("./App.tsx", { platform: "linux", width: 1100, height: 720 });

test("starts on the agents page and asks for a provider first", async () => {
  const app = await launch();
  await app.waitFor("id:page-agents");
  expect(app.texts()).toContain("Let your agents use the models you set up in pansy");
  await app.press("id:nav-providers");
  await app.waitFor("text:No providers yet");
  expect(app.texts()).toContain("Providers");
});

test("says an agent's update, runs it when asked, and moves a row dragged by its logo", async () => {
  const app = await launch();
  await app.waitFor("id:agent-codex");
  // test.sh's Codex is an npm package a newer version is out for, by its registry
  await app.waitFor("id:update-codex");
  expect(app.texts()).toContain("0.168.0");
  expect(app.texts()).toContain("Update to 0.170.0");
  await app.press("id:update-codex");
  await app.waitUntil(() => app.texts().includes("0.170.0") && !app.texts().includes("Update to 0.170.0"));
  expect(app.texts()).toContain("Codex updated: 0.168.0 → 0.170.0");

  // Claude Code first, as pansy lists them; Codex dragged over it goes first
  const order = () => app.texts().filter((t) => t === "Claude Code" || t === "Codex");
  expect(order()).toEqual(["Claude Code", "Codex"]);
  await app.drag("id:handle-codex", { y: -50 });
  await app.waitUntil(() => order()[0] === "Codex");
  await app.drag("id:handle-codex", { y: 50 });
  await app.waitUntil(() => order()[0] === "Claude Code");
});

test("adds a provider of the user's own, then a group of its models", async () => {
  const app = await launch();
  await app.press("id:nav-providers");
  await app.press("id:provider-add");
  await app.waitFor("id:add-provider");
  await app.press("id:add-custom");
  await app.waitFor("id:custom-modal");
  await app.type("id:custom-name", "Local relay");
  await app.type("id:custom-url", "http://127.0.0.1:9/v1");
  await app.type("id:custom-models", "model-a, model-b");
  await app.press("id:custom-add");
  await app.waitFor("id:provider-local-relay");
  expect(app.texts()).toContain("Local relay");

  await app.press("id:provider-open-local-relay");
  await app.waitFor("id:provider-editor");
  await app.waitFor("text:model-a");
  await app.press("id:provider-done");

  await app.press("id:nav-groups");
  await app.press("id:group-new");
  await app.waitFor("id:group-editor");
  await app.type("id:group-name", "Daily");
  await app.press("id:group-add-member");
  await app.waitFor("id:picker");
  await app.press("id:pick-local-relay/model-a");
  await app.press("id:group-add-member");
  await app.waitFor("id:picker");
  await app.press("id:pick-local-relay/model-b");
  await app.press("id:group-routing-rotate");
  await app.press("id:group-save");
  await app.waitFor("id:group-daily");
  expect(app.texts()).toContain("group/daily");
});

test("adds a vendor at one of its plans, then moves it to another", async () => {
  const app = await launch();
  await app.press("id:nav-providers");
  await app.press("id:provider-add");
  await app.waitFor("id:add-provider");
  // Volcengine Ark, by the words of its note, with magpie's logo and plans
  await app.type("id:preset-search", "火山");
  await app.waitFor("id:preset-volcengine");
  await app.press("id:preset-volcengine");
  await app.waitFor("id:preset-modal");
  expect(app.texts()).toContain("Coding Plan");
  await app.press("id:preset-region-agent");
  await app.waitFor("text:https://ark.cn-beijing.volces.com/api/plan/v3");
  await app.type("id:preset-key", "sk-ark-test");
  await app.press("id:preset-add");
  await app.waitFor("id:provider-volcengine");
  // The Agent Plan's models, which its endpoints don't list
  await app.waitFor("text:ark.cn-beijing.volces.com · 15 models");

  await app.press("id:provider-volcengine");
  await app.waitFor("id:provider-editor");
  await app.press("id:provider-region-volcengine-coding");
  await app.waitFor("text:https://ark.cn-beijing.volces.com/api/coding");
  await app.press("id:provider-done");

  // A vendor's global and China endpoints are one row, picked in the dialog
  await app.press("id:provider-add");
  await app.clear("id:preset-search");
  await app.type("id:preset-search", "kimi");
  await app.waitFor("id:preset-moonshot");
  expect(app.texts()).not.toContain("Kimi (China)");
  await app.press("id:preset-moonshot");
  await app.press("id:preset-area-moonshot-cn");
  await app.waitFor("text:https://api.moonshot.cn/v1");
});

test("offers the subscriptions to sign in to, in the browser or by a code", async () => {
  const app = await launch();
  await app.press("id:nav-providers");
  await app.press("id:provider-add");
  await app.waitFor("id:add-subscriptions");
  await app.press("id:subscription-codex");
  await app.waitFor("id:sign-in-modal");
  expect(app.texts()).toContain("Sign in to ChatGPT");
  expect(app.texts()).toContain("ChatGPT's page opens in your browser and comes back to pansy");
  await app.press("id:sign-in-how-code");
  await app.waitFor("text:For a browser that can't come back to this computer: type a code on ChatGPT's page");
  await app.press("id:sign-in-cancel");
  expect(app.texts()).not.toContain("Sign in to ChatGPT");
  // GitHub signs in by a code alone.
  await app.press("id:subscription-copilot");
  await app.waitFor("id:sign-in-modal");
  expect(app.texts()).toContain("Sign in to GitHub Copilot");
  expect(app.texts()).not.toContain("In the browser");
  await app.press("id:sign-in-cancel");
  // Google in the browser alone.
  await app.press("id:subscription-gemini-cli");
  await app.waitFor("id:sign-in-modal");
  expect(app.texts()).toContain("Sign in to Gemini Code Assist");
  expect(app.texts()).not.toContain("With a code");
  await app.press("id:sign-in-cancel");
});

test("waits on the Routing page for a request, with the group's accounts below", async () => {
  const app = await launch();
  await app.press("id:nav-routing");
  await app.waitFor("id:page-routing");
  await app.waitFor("text:Waiting for a request");
  expect(app.texts()).toContain("your agent");
  expect(app.texts()).toContain("No request yet");
  // Daily's models, from the test before, with their tries today
  await app.waitFor("id:routing-acts-local-relay");
  expect(app.texts()).toContain("model-a");
  await app.press("id:routing-hide");
  await app.waitFor("text:Show accounts");
});

test("shows what connecting an agent would write, and writes nothing by looking", async () => {
  const app = await launch();
  await app.waitFor("id:agent-claude-code");
  await app.press("id:preview-claude-code");
  await app.waitFor("id:preview");
  await app.waitFor("text:+");
  const said = app.texts().join("\n");
  expect(said).toContain("What connecting Claude Code changes");
  expect(said).toContain("env.ANTHROPIC_BASE_URL = http://127.0.0.1:7267");
  expect(said).toContain("Nothing is written until you connect.");
  await app.press("id:preview-close");
  await app.waitFor("id:status-claude-code");
  expect(app.texts().join("\n")).toContain("Not connected · Claude Code uses its own settings");
});

test("gives one key a proxy of its own", async () => {
  const app = await launch();
  await app.press("id:nav-providers");
  await app.press("id:provider-open-local-relay");
  await app.waitFor("id:provider-editor");
  await app.type("id:key-new-local-relay", "sk-proxy-test-0123456789");
  await app.press("id:key-add-local-relay");
  await app.waitFor("id:key-proxy-open-local-relay-0");
  await app.press("id:key-proxy-open-local-relay-0");
  await app.waitFor("id:key-proxy-local-relay");
  await app.type("id:key-proxy-local-relay", "socks5://127.0.0.1:1080");
  await app.press("id:key-proxy-local-relay-save");
  await app.waitFor("text:Proxy saved");
  await app.press("id:provider-done");
});

test("connects an agent to a model with its switch and disconnects it", async () => {
  const app = await launch();
  await app.press("id:nav-agents");
  await app.waitFor("id:agent-claude-code");
  // test.sh's claude says its version, asked after the rows
  await app.waitFor("id:version-claude-code");
  expect(app.texts()).toContain("2.1.0");
  await app.press("id:connect-claude-code");
  await app.waitFor("id:picker");
  await app.type("id:picker-search", "model-b");
  await app.press("id:pick-local-relay/model-b");
  await app.waitUntil(() => app.texts().includes("Connected · switch the opus · sonnet · haiku tiers in /model"));
  expect(app.texts()).toContain("model-b");

  // Details: where it picks, and what pansy changed in its settings.json
  await app.press("id:details-claude-code");
  await app.waitFor("text:In Claude Code");
  await app.press("id:changes-claude-code");
  await app.waitUntil(() => app.texts().some((t) => t.includes("env.ANTHROPIC_BASE_URL = http://127.0.0.1:")));
  expect(app.texts().join(" ")).toContain("env.ANTHROPIC_MODEL = local-relay/model-b");

  // Its tiers, each on the main model until it is given its own; a tier's
  // effort goes after its model's name, the session's is a key of its own
  const wrote = (line: string) => app.texts().some((t) => t.includes(line));
  await app.waitFor("id:tiers-claude-code");
  expect(app.texts()).toContain("/model opus");
  expect(app.texts()).toContain("same as model");
  await app.press("id:field-claude-code-opus");
  await app.waitFor("id:picker");
  await app.type("id:picker-search", "model-a");
  await app.press("id:pick-local-relay/model-a");
  await app.waitUntil(() => wrote("env.ANTHROPIC_DEFAULT_OPUS_MODEL = local-relay/model-a"));
  await app.press("id:effort-claude-code-opus_effort");
  await app.waitFor("id:effort-picker");
  await app.press("id:effort-top");
  await app.waitUntil(() => wrote("env.ANTHROPIC_DEFAULT_OPUS_MODEL = local-relay/model-a:max"));
  await app.press("id:effort-claude-code-effort");
  await app.waitFor("id:effort-picker");
  await app.press("id:effort-top");
  await app.waitUntil(() => wrote("effortLevel = max"));
  await app.press("id:effort-claude-code-effort");
  await app.waitFor("id:effort-picker");
  await app.press("id:effort-default");
  await app.waitUntil(() => !wrote("effortLevel"));
  await app.press("id:field-claude-code-opus");
  await app.waitFor("id:pick-reset");
  await app.press("id:pick-reset");
  await app.waitUntil(() => wrote("env.ANTHROPIC_DEFAULT_OPUS_MODEL = local-relay/model-b:max"));

  // The model its subagents start on: a key more in its settings, then none
  await app.waitFor("id:subagents-claude-code");
  expect(app.texts()).toContain("left to Claude Code");
  await app.press("id:subagent-claude-code");
  await app.waitFor("id:picker");
  await app.type("id:picker-search", "model-a");
  await app.press("id:pick-local-relay/model-a");
  await app.waitUntil(() => app.texts().some((t) => t.includes("env.CLAUDE_CODE_SUBAGENT_MODEL = local-relay/model-a")));
  await app.press("id:subagent-claude-code");
  await app.waitFor("id:pick-reset");
  await app.press("id:pick-reset");
  await app.waitUntil(() => app.texts().includes("left to Claude Code"));
  expect(app.texts().join(" ")).not.toContain("CLAUDE_CODE_SUBAGENT_MODEL");

  // Off asks first, with what goes back
  await app.press("id:connect-claude-code");
  await app.waitFor("id:disconnect");
  await app.waitFor("text:Disconnect Claude Code from pansy?");
  await app.press("id:disconnect-confirm");
  await app.waitUntil(() => app.texts().includes("Not connected · Claude Code uses its own settings"));
});

test("checks the agents in Diagnostics, and restores one for now and connects it again", async () => {
  const app = await launch();
  await app.press("id:nav-agents");
  // On again with the model it had
  await app.press("id:connect-claude-code");
  await app.waitUntil(() => app.texts().includes("Connected · switch the opus · sonnet · haiku tiers in /model"));

  await app.press("id:nav-settings");
  await app.press("id:settings-tab-diagnostics");
  await app.waitFor("id:diagnosis-claude-code-agent-connected");
  expect(app.texts()).toContain("Claude Code is on local-relay/model-b");
  await app.waitFor("id:diagnosis-providers-providers-on");
  // A test request is ready for the first model; not sent: the port may be another pansy's
  await app.waitFor("id:test-send");
  expect(app.texts()).toContain("Send a test request");

  // One, from its finding
  await app.press("id:fix-claude-code-agent-connected");
  await app.waitFor("id:diagnosis-claude-code-agent-paused");
  await app.press("id:fix-claude-code-agent-paused");
  await app.waitFor("id:diagnosis-claude-code-agent-connected");

  // Every one, asked first; the Agents page says so
  await app.press("id:diagnosis-pause-all");
  await app.waitFor("text:Restore every agent's own config for now?");
  await app.press("id:question-confirm");
  await app.waitFor("id:diagnosis-claude-code-agent-paused");
  await app.press("id:nav-agents");
  await app.waitUntil(() => app.texts().includes("Restored for now · Claude Code uses its own settings until it is connected again"));
  await app.press("id:nav-settings");
  await app.press("id:diagnosis-resume-all");
  await app.waitFor("id:diagnosis-claude-code-agent-connected");

  // Disconnected, as the tests after expect it
  await app.press("id:nav-agents");
  await app.press("id:connect-claude-code");
  await app.press("id:disconnect-confirm");
  await app.waitUntil(() => app.texts().includes("Not connected · Claude Code uses its own settings"));
});

test("hides an agent with Manage, and shows it again", async () => {
  const app = await launch();
  await app.waitFor("id:agent-claude-code");
  await app.press("id:agents-manage-open");
  await app.waitFor("id:agents-manage");
  await app.press("id:manage-claude-code");
  await app.press("id:manage-hide");
  await app.waitFor("text:1 agent hidden");
  await app.press("id:manage-done");
  await app.waitUntil(() => app.texts().includes("1 hidden agents"));
  await app.press("id:agents-more");
  await app.waitFor("id:show-claude-code");
  await app.press("id:show-claude-code");
  await app.waitUntil(() => !app.texts().includes("1 hidden agents"));
  await app.waitFor("id:connect-claude-code");
});

test("shows the gateway and settings in Chinese", async () => {
  const app = await launch();
  await app.press("id:nav-gateway");
  await app.waitFor("id:page-gateway");
  expect(app.texts()).toContain("Share on the local network");
  await app.press("id:nav-settings");
  await app.waitFor("id:page-settings");
  await app.press("id:setting-language");
  await app.press("id:setting-language-zh-Hans");
  await app.waitFor("text:设置");
  expect(app.texts()).toContain("外观");
  await app.press("id:setting-language");
  await app.press("id:setting-language-en");
  await app.waitFor("text:Appearance");
});

test("finds a setting on any of Settings' pages", async () => {
  const app = await launch();
  await app.press("id:nav-settings");
  await app.waitFor("id:settings-nav");
  await app.press("id:settings-tab-network");
  await app.waitFor("text:Model ids without the provider");
  await app.type("id:settings-find", "whole words");
  await app.waitFor("text:Whole words only");
  expect(app.texts()).not.toContain("Model ids without the provider");
  await app.clear("id:settings-find");
  await app.type("id:settings-find", "nothing like this");
  await app.waitFor("id:settings-none");
});

test("opens the Plugins market, and Installed with none added yet", async () => {
  const app = await launch();
  await app.press("id:nav-plugins");
  await app.waitFor("id:page-plugins");
  await app.waitFor("id:plugins-tab");
  await app.waitFor("id:market-find");
  await app.waitFor("id:plugins-mirror");
  await app.press("id:plugins-tab-installed");
  await app.waitFor("text:No plugins yet");
  await app.press("id:plugins-to-discover");
  await app.waitFor("id:plugins-tab-installed");
});

test("says in About that a build without a release feed doesn't look for updates", async () => {
  const app = await launch();
  await app.press("id:nav-settings");
  await app.waitFor("id:settings-nav");
  await app.press("id:settings-tab-about");
  await app.waitFor("id:setting-row-updates");
  await app.waitFor("text:This build doesn't look for updates: it has no release feed");
  // no Update in the title bar, nor Check for updates, without one
  await app.waitFor("id:update", { state: "gone" });
  await app.waitFor("id:update-check", { state: "gone" });
});

test("shows What's new once after an update, and again from About", async () => {
  // the version running, as About says it
  const first = await launch();
  await first.press("id:nav-settings");
  await first.press("id:settings-tab-about");
  await first.waitFor("id:setting-row-whats-new");
  await first.waitFor("text:No release notes yet: they come with an update");
  const version = first.texts().find((text) => text.startsWith("Version "))!.slice("Version ".length);
  // the notes the update before the restart kept
  const notes = { version, text: "## Fixed\n- The gateway **keeps** a stream open\nThanks for the reports." };
  const app = await open("./App.tsx", { platform: "linux", width: 1100, height: 720, storage: { "update.notes": JSON.stringify(notes) } });
  await app.waitFor("id:whats-new");
  await app.waitFor(`text:What's new in ${version}`);
  expect(app.texts()).toContain("Fixed");
  expect(app.texts()).toContain("The gateway keeps a stream open");
  expect(app.texts()).toContain("Thanks for the reports.");
  expect(app.texts()).not.toContain("Not installed yet");
  await app.press("id:whats-new-close");
  await app.waitFor("id:whats-new", { state: "gone" });
  // shown once: kept as seen
  await app.waitUntil(() => JSON.parse(app.storage()["update.notes"]).seen === true);
  await app.press("id:nav-settings");
  await app.press("id:settings-tab-about");
  await app.press("id:whats-new-open");
  await app.waitFor("id:whats-new");
  // notes of another version are not this one's
  const other = await open("./App.tsx", { platform: "linux", width: 1100, height: 720, storage: { "update.notes": JSON.stringify({ ...notes, version: "0.0.1" }) } });
  await other.waitFor("id:page-agents");
  await other.waitFor("id:whats-new", { state: "gone" });
});

test("masks a key in Try it, and opens the Masking page", async () => {
  const app = await launch();
  await app.press("id:nav-masking");
  await app.waitFor("id:masking-empty");
  await app.press("id:masking-on");
  await app.waitFor("id:masking-summary");
  await app.press("id:nav-settings");
  await app.press("id:settings-tab-privacy");
  await app.waitFor("text:Mask secrets");
  await app.type("id:mask-try", "the key is sk-proj-Q2xhdWRlIGNvZGUgbWFza3MgdGhpcyBrZXk0V3Nz");
  await app.waitUntil(() => app.texts().some((t) => t.includes("{{API_KEY_")));
  expect(app.texts().join(" ")).not.toContain("Q2xhdWRlIGNvZGUg");
});

test("brings a provider over from magpie, and offers it no more", async () => {
  const app = await launch();
  await app.waitFor("id:page-agents");
  await app.press("id:nav-providers");
  await app.waitFor("id:import-found-review");
  await app.press("id:import-found-review");
  await app.waitFor("id:import-modal");
  await app.waitFor("id:import-magpie-magpie-relay");
  expect(app.texts()).toContain("Magpie Relay");
  expect(app.texts()).toContain("relay.example.com · sk-m••••6789 · 1 model");
  await app.waitFor("id:import-go");
  await app.press("id:import-go");
  await app.waitFor("id:provider-magpie-relay");
  await app.waitFor("id:import-found-review", { state: "gone" });
  // the import again says it is here
  await app.press("id:provider-add");
  await app.press("id:add-import");
  await app.waitFor("id:import-magpie-magpie-relay");
  await app.waitFor("text:Already added");
});

test("fills a custom provider from pasted environment lines, and adds what a pansy:// link describes", async () => {
  const app = await launch();
  await app.waitFor("id:page-agents");
  await app.press("id:nav-providers");
  await app.press("id:provider-add");
  await app.press("id:add-paste");
  await app.waitFor("id:paste-modal");
  await app.type("id:paste-text", 'export ANTHROPIC_BASE_URL="https://api.pasted.example/anthropic"\nexport ANTHROPIC_AUTH_TOKEN="sk-pasted-0123456789"');
  await app.waitFor("text:Filled in from the pasted text: Base URL (Anthropic) · API key.");
  await app.press("id:paste-go");
  await app.waitFor("id:custom-modal");
  expect(app.value("id:custom-name")).toBe("Pasted");
  expect(app.value("id:custom-url")).toBe("https://api.pasted.example/anthropic");
  await app.press("id:custom-add");
  await app.waitFor("text:Pasted added");

  // a link the system hands the app: shown first, added on the yes
  await app.openWithUrl("pansy://import?name=Link%20Relay&chat=https://link.example.com/v1&key=sk-link-0123456789&models=lr-1");
  await app.waitFor("id:link-modal");
  expect(app.texts()).toContain("Added from a link. Your prompts and this key will go to link.example.com; add it only if you trust the site that sent you here.");
  expect(app.texts().join(" ")).not.toContain("sk-link-0123456789");
  await app.press("id:link-add");
  await app.waitFor("id:provider-link-relay");
  // one that can't be imported says why
  await app.openWithUrl("pansy://import?name=Bare");
  await app.waitFor("id:link-error");
});

test("lists a session, shows what was said, and deletes it into the trash and back", async () => {
  // test.sh's Claude Code session
  const id = "11111111-2222-4333-8444-555555555555";
  const app = await launch();
  await app.press("id:nav-sessions");
  await app.waitFor("id:page-sessions");
  await app.waitFor("id:sessions-agent-opencode");
  await app.press("id:sessions-agent-claude-code");
  await app.waitFor(`id:session-${id}`);
  expect(app.texts()).toContain("Login bug fix");
  await app.press(`id:session-open-${id}`);
  await app.waitFor(`id:session-details-${id}`);
  expect(app.texts()).toContain(`cd '/work/app' && claude --resume ${id}`);

  // saved as Markdown into the scratch home's Downloads
  await app.press(`id:session-export-${id}`);
  await app.waitFor("text:Saved to ~/Downloads/pansy-session-claude-code-11111111-222.md");

  await app.press(`id:session-talk-${id}`);
  await app.waitFor("id:session-talk");
  // where it is among its prompts: its one
  await app.waitFor("id:session-nav-count");
  expect(app.texts()).toContain("1 / 1");
  await app.waitFor("text:The form now trims the e-mail before it checks it.");
  expect(app.texts()).toContain("Fix the login bug");
  // a tool call's card: how it ended, and opened, what it was given and gave back
  await app.waitFor("id:session-tool-1");
  expect(app.texts()).toContain("Completed");
  await app.press("id:session-tool-1");
  await app.waitUntil(() => app.texts().some((t) => t.includes("export function login")));
  await app.press("id:sessions-back");

  // found by what was said in it, not its title
  await app.waitFor("id:sessions-search");
  await app.type("id:sessions-search", "trims the");
  await app.waitUntil(() => app.texts().some((t) => t.includes("before it checks it")));
  await app.clear("id:sessions-search");

  await app.press(`id:session-delete-${id}`);
  await app.waitFor("id:question");
  await app.press("id:question-confirm");
  // Claude Code has none left: OpenCode's, which are listed, not deleted
  await app.waitFor("text:Health check endpoint");
  expect(app.texts().join(" ")).toContain("its sessions are listed and resumed here, not deleted");
  await app.press("id:sessions-trash");
  await app.waitFor(`id:trashed-${id}`);
  await app.press(`id:trash-restore-${id}`);
  await app.waitFor("id:trash-none");
  await app.press("id:trash-back");
  await app.waitFor(`id:session-${id}`);
});

test("hides the sidebar for tabs in the title bar, and shows it again", async () => {
  const app = await launch();
  await app.waitFor("id:page-agents");
  await app.waitFor("id:brand-mark");
  await app.press("id:side-toggle");
  await app.waitFor("id:nav-tabs");
  await app.waitFor("id:nav-agents", { state: "gone" });
  await app.screenshot("/tmp/pansy-side-hidden.png");
  await app.press("id:tab-providers");
  await app.waitFor("text:No providers yet");
  // Settings' pages and Find a setting are on the page while the sidebar is away
  await app.press("id:tab-settings");
  await app.press("id:settings-tabs-network");
  await app.waitFor("text:Model ids without the provider");
  await app.screenshot("/tmp/pansy-side-hidden-settings.png");
  // the choice is kept for the next launch, and the keystroke brings it back
  const again = await launch();
  await again.waitFor("id:nav-tabs");
  await again.keyboard.press("ctrl+shift+s");
  await again.waitFor("id:nav-agents");
  await again.waitFor("id:nav-tabs", { state: "gone" });
  await again.screenshot("/tmp/pansy-side-shown.png");
});
