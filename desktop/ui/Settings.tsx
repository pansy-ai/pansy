// Settings, as magpie has them: pages (General, Network and sharing,
// Privacy, Diagnostics, About) under Settings in the sidebar, each a list of cards of
// rows — a setting's name and the line under it on the left, its control on
// the right. "Find a setting" in the title bar shows every page's settings
// that match. Every setting is a row below, in the order the pages show
// them; a row's control is made once and follows the settings itself.
import { Clipboard, signal, Text, token, untracked, View, type JSX } from "nsh";
import { Masking, Settings } from "pansy/native";
import { LANGUAGES, language, setLanguage, tr, trn, type Choice } from "./i18n";
import {
  categoryLabel,
  gateway,
  go,
  kindLabel,
  MASK_CATEGORIES,
  maskingOn,
  PERSONAL_KINDS,
  reason,
  SECRET_KINDS,
  setMasking,
  setSetting,
  settings,
  settingsQuery,
  settingsTab,
  sideShown,
  SIDEBAR_PAGES,
  toast,
  type MaskAction,
  type Page,
  type SettingsTab,
} from "./model";
import { Badge, Dropdown, FieldButton, Icon, IconButton, Input, List, MOTION, Mono, OnOff, Page as PageView, PrimaryButton, RowHead, Segs, SettingRow, SHADOW, TextButton } from "./parts";
import { hasNotes, openNotes, UpdateControl, updateSaid, versionSaid } from "./Update";
import { FindingList, FromATerminal, HealthControl, healthSaid, RestoreAll, ResumeAll, TestControl, TestResult, testSaid } from "./Diagnostics";
import { Menu, MenuItem } from "nsh/base";

/** Each page of Settings: its name and icon in the sidebar. */
export const SETTINGS_TABS = (): { id: SettingsTab; name: string; icon: "sliders-horizontal" | "globe" | "shield" | "activity" | "info" }[] => [
  { id: "general", name: tr("General"), icon: "sliders-horizontal" },
  { id: "network", name: tr("Network and sharing"), icon: "globe" },
  { id: "privacy", name: tr("Privacy"), icon: "shield" },
  { id: "diagnostics", name: tr("Diagnostics"), icon: "activity" },
  { id: "about", name: tr("About"), icon: "info" },
];

/** One setting: where it is, what it is called and says, and its control. */
interface Setting {
  id: string;
  tab: SettingsTab;
  /** The card it is in: the small capitals above it, none for the page's first. */
  section: () => string;
  title: () => string;
  description?: () => string;
  /** Shown only while this holds (a category of personal data while personal data is masked). */
  when?: () => boolean;
  /** On the right of the name. */
  control?: () => JSX.Element;
  /** Under the name, the row's width. */
  below?: () => JSX.Element;
  /** Beside the name: NEW, Always on. */
  badge?: () => JSX.Element;
}

const s = () => settings();
const m = () => settings()?.masking;

// --- Lists the user edits: words, values let go, web pages ----------------------

/**
 * A list of the user's values, one on each row: a field and Add, the values
 * under it, each removed with its ×. `hide` shows them as dots until Show.
 */
function ValueList(props: { values: string[]; onChange: (values: string[]) => void; placeholder: string; testID: string; hide?: boolean; mono?: boolean }) {
  const draft = signal("");
  const shown = signal(!props.hide);
  const add = () => {
    const parts = draft()
      .split("\n")
      .map((v) => v.trim())
      .filter((v) => v && !props.values.includes(v));
    if (parts.length === 0) return;
    props.onChange([...props.values, ...parts]);
    draft.set("");
  };
  return (
    <View class="gap-[7px]">
      <View class="flex-row items-center gap-2">
        <Input value={draft()} onChange={(v) => draft.set(v)} onSubmit={add} placeholder={props.placeholder} mono={props.mono} testID={`${props.testID}-input`} class="flex-1" />
        <TextButton label={tr("Add")} onPress={add} disabled={!draft().trim()} testID={`${props.testID}-add`} />
      </View>
      <If when={props.values.length > 0}>
        <View class="flex-row flex-wrap gap-[6px]" testID={props.testID}>
          <Each of={props.values} key={(v) => v}>
            {(v) => (
              <View class="h-[22px] flex-row items-center gap-1 rounded-[6px] bg-pill pl-[8px] pr-[3px]">
                <Text class={`text-[12px] text-fg-2 ${props.mono ? "font-mono" : ""}`}>{shown() ? v() : "•".repeat(Math.min(v().length, 12))}</Text>
                <View
                  role="button"
                  accessibilityLabel={tr("Remove %s", shown() ? v() : "")}
                  onPress={() => props.onChange(props.values.filter((x) => x !== v()))}
                  class={`group h-[16px] w-[16px] items-center justify-center rounded-[4px] hover:bg-pill-hover active:scale-90 ${MOTION}`}
                >
                  <Icon name="x" color="muted-foreground" hover="foreground" size={10} />
                </View>
              </View>
            )}
          </Each>
        </View>
      </If>
      <If when={props.hide && props.values.length > 0}>
        <View class="flex-row gap-1">
          <TextButton label={shown() ? tr("Hide") : tr("Show")} tone="muted" onPress={() => shown.set(!shown())} testID={`${props.testID}-show`} />
          <TextButton
            label={tr("Copy")}
            tone="muted"
            onPress={() => void Clipboard.writeText(props.values.join("\n")).then(() => toast(tr("Copied")))}
          />
        </View>
      </If>
    </View>
  );
}

/** The user's masking rules: Prefix or Regex, the value, Add; the rules under it. */
function RuleList() {
  const kind = signal<"prefix" | "regex">("prefix");
  const draft = signal("");
  const rules = () => m()?.rules ?? [];
  const add = () => {
    const value = draft().trim();
    if (!value) return;
    void setMasking({ rules: [...rules(), { kind: kind(), value }] }).then((ok) => ok && draft.set(""));
  };
  return (
    <View class="gap-[7px]">
      <View class="flex-row items-center gap-2">
        <Segs
          small
          testID="mask-rule-kind"
          value={kind()}
          onChange={(v) => kind.set(v)}
          options={[
            { value: "prefix", label: tr("Prefix") },
            { value: "regex", label: tr("Regex") },
          ]}
        />
        <Input value={draft()} onChange={(v) => draft.set(v)} onSubmit={add} mono placeholder={kind() === "prefix" ? "oc_sk_" : "acme-[0-9a-f]{32}"} testID="mask-rule-input" class="flex-1" />
        <TextButton label={tr("Add")} onPress={add} disabled={!draft().trim()} testID="mask-rule-add" />
      </View>
      <If when={rules().length > 0}>
        <View class="overflow-hidden rounded-[8px] border border-line-2">
          <Each of={rules()} key={(r) => `${r.kind}:${r.value}`}>
            {(r, i) => (
              <View class={`h-[30px] flex-row items-center gap-2 pl-[9px] pr-[4px] ${i() === 0 ? "" : "border-t border-line-2"}`}>
                <Badge label={r().kind === "prefix" ? tr("Prefix") : tr("Regex")} />
                <Mono text={r().value} class="flex-1 text-[11.5px] line-clamp-1" />
                <Text class="font-mono text-[10.5px] text-faint">{`{{${r().name || "CUSTOM"}_…}}`}</Text>
                <IconButton icon="x" size={11} label={tr("Remove")} onPress={() => void setMasking({ rules: rules().filter((_, j) => j !== i()) })} />
              </View>
            )}
          </Each>
        </View>
      </If>
    </View>
  );
}

/** Text masked as a request would be, here and nowhere else. */
function TryIt() {
  const text = signal("");
  const out = signal<{ text: string; hits: { placeholder: string; kind: string; action: string }[] } | null>(null);
  let turn = 0;
  const run = (value: string) => {
    text.set(value);
    const mine = ++turn;
    if (!value.trim()) {
      out.set(null);
      return;
    }
    Masking.tryIt(value)
      .then((json) => {
        if (mine === turn) out.set(JSON.parse(json));
      })
      .catch((error) => toast(reason(error), "error"));
  };
  return (
    <View class="gap-2">
      <Input multiline value={text()} onChange={run} mono placeholder={tr("My key is sk-proj-… and my email is ada@example.com")} testID="mask-try" />
      <If when={out()}>
        <View class="gap-[6px] rounded-[8px] bg-card-2 px-[10px] py-2" style={{ boxShadow: `inset 0 0 0 1px ${token("line-2")}` }}>
          <Text testID="mask-try-out" class="font-mono text-[11.5px] text-foreground">
            {out()?.text ?? ""}
          </Text>
          <Text class="text-[11px] text-muted-foreground">
            {out()?.hits.length
              ? trn(out()!.hits.length, "%d value masked: %s", "%d values masked: %s", out()!.hits.length, [...new Set(out()!.hits.map((h) => kindLabel(h.kind)))].join(", "))
              : maskingOn()
                ? tr("Nothing to mask in it")
                : tr("Masking is off: turn on Mask secrets or Mask personal data")}
          </Text>
        </View>
      </If>
    </View>
  );
}

/** Sidebar pages: a menu of the pages, each shown or hidden. */
function SidebarPages() {
  const NAMES = (): Record<Page, string> => ({
    agents: tr("Agents"),
    sessions: tr("Sessions"),
    providers: tr("Providers"),
    routing: tr("Routing"),
    groups: tr("Routing groups"),
    gateway: tr("Gateway"),
    plugins: tr("Plugins"),
    usage: tr("Usage"),
    masking: tr("Masking"),
    settings: tr("Settings"),
  });
  const hidden = () => s()?.hiddenPages ?? [];
  const label = () => (hidden().length === 0 ? tr("All shown") : trn(hidden().length, "%d hidden", "%d hidden", hidden().length));
  return (
    <Menu
      placement="bottom"
      align="end"
      offset={4}
      trigger={<FieldButton value={label()} width={150} testID="setting-pages" />}
      popupClass="min-w-[180px] rounded-[10px] border border-border bg-popover p-1"
      popupStyle={{ boxShadow: SHADOW.pop() }}
    >
      <Each of={SIDEBAR_PAGES} key={(p) => p}>
        {(p) => (
          <MenuItem
            role="menuitemcheckbox"
            checked={!hidden().includes(p())}
            closeOnSelect={false}
            testID={`setting-pages-${p()}`}
            onSelect={() => {
              const next = hidden().includes(p()) ? hidden().filter((x) => x !== p()) : [...hidden(), p()];
              void setSetting("hiddenPages", next);
            }}
            class="h-[28px] flex-row items-center gap-2 rounded-[7px] px-2 hover:bg-pill focus:bg-pill"
          >
            <View class="w-3">
              <If when={!hidden().includes(p())}>
                <Icon name="tick" color="primary" size={12} />
              </If>
            </View>
            <Text class="text-[12.5px] text-foreground">{NAMES()[p()]}</Text>
          </MenuItem>
        )}
      </Each>
    </Menu>
  );
}

/** The proxy's mode: the system's (Auto), none (Off), or one of the user's (Custom). */
const proxyMode = (): "auto" | "off" | "custom" => {
  const p = s()?.proxy ?? "";
  return p === "" ? "auto" : p === "direct" ? "off" : "custom";
};
/** Custom chosen, its address not saved yet. */
const proxyEditing = signal(false);

function ProxyControl() {
  return (
    <Segs
      testID="setting-proxy"
      value={proxyEditing() ? "custom" : proxyMode()}
      onChange={(v) => {
        if (v === "custom") {
          proxyEditing.set(true);
          return;
        }
        proxyEditing.set(false);
        void setSetting("proxy", v === "auto" ? "" : "direct");
      }}
      options={[
        { value: "auto", label: tr("Auto") },
        { value: "off", label: tr("Off") },
        { value: "custom", label: tr("Custom") },
      ]}
    />
  );
}

function ProxyAddress() {
  const draft = signal<string | null>(null);
  const value = () => draft() ?? (proxyMode() === "custom" ? (s()?.proxy ?? "") : "");
  const save = () => {
    const v = value().trim();
    if (!v) return;
    void setSetting("proxy", v).then((ok) => {
      if (!ok) return;
      draft.set(null);
      proxyEditing.set(false);
    });
  };
  return (
    <View class="flex-row items-center gap-2">
      <Input value={value()} onChange={(v) => draft.set(v)} onSubmit={save} mono autoFocus={proxyEditing()} placeholder="http://127.0.0.1:7890 · socks5://127.0.0.1:1080" testID="setting-proxy-url" class="flex-1" />
      <If when={!!value().trim() && value().trim() !== s()?.proxy}>
        <PrimaryButton label={tr("Save")} onPress={save} testID="setting-proxy-save" />
      </If>
    </View>
  );
}

function PortControl() {
  const draft = signal<string | null>(null);
  const apply = () => {
    const n = Number(draft());
    if (!Number.isInteger(n)) return;
    void setSetting("port", n, true).then((ok) => ok && draft.set(null));
  };
  return (
    <View class="flex-row items-center gap-2">
      <Input value={draft() ?? String(s()?.port ?? "")} onChange={(v) => draft.set(v)} onSubmit={apply} mono testID="setting-port" class="w-[80px]" />
      <TextButton label={tr("Apply")} onPress={apply} disabled={draft() === null || draft() === String(s()?.port)} testID="setting-port-apply" />
    </View>
  );
}

/** What is done with a kind of value found: masked, sent and told, or the request held back. */
function ActionControl(props: { kind: string }) {
  const value = () => m()?.actions[props.kind] ?? "mask";
  return (
    <Segs
      small
      testID={`mask-action-${props.kind}`}
      value={value()}
      onChange={(v: MaskAction) => {
        const actions = { ...(m()?.actions ?? {}) };
        if (v === "mask") delete actions[props.kind];
        else actions[props.kind] = v;
        void setMasking({ actions });
      }}
      options={[
        { value: "mask", label: tr("Mask") },
        { value: "warn", label: tr("Warn") },
        { value: "block", label: tr("Block") },
      ]}
    />
  );
}

const actionText = (kind: string) => {
  switch (m()?.actions[kind] ?? "mask") {
    case "warn":
      return tr("Goes to the vendor as written, and Masking notes it");
    case "block":
      return tr("The request is held back, and the agent told why");
  }
  return tr("Goes to the vendor as a placeholder, and comes back as it was");
};

function ActionRows(kinds: string[], when: () => boolean): Setting[] {
  return kinds.map((kind) => ({
    id: `action-${kind}`,
    tab: "privacy" as const,
    section: () => tr("When a value is found"),
    title: () => kindLabel(kind),
    description: () => actionText(kind),
    when,
    control: () => <ActionControl kind={kind} />,
  }));
}

// --- Every setting ------------------------------------------------------------

const SETTINGS: Setting[] = [
  // General
  {
    id: "theme",
    tab: "general",
    section: () => "",
    title: () => tr("Appearance"),
    control: () => (
      <Segs
        testID="setting-theme"
        value={s()?.theme ?? "system"}
        onChange={(v) => void setSetting("theme", v)}
        options={[
          { value: "system", label: tr("System") },
          { value: "light", label: tr("Light") },
          { value: "dark", label: tr("Dark") },
        ]}
      />
    ),
  },
  {
    id: "language",
    tab: "general",
    section: () => "",
    title: () => tr("Language"),
    control: () => (
      <Dropdown
        testID="setting-language"
        value={language()}
        onChange={(v: Choice) => {
          try {
            setLanguage(v);
          } catch (error) {
            toast(String(error), "error");
          }
        }}
        options={LANGUAGES.map((l) => ({ value: l.choice, label: l.choice === "system" ? tr("System") : l.name }))}
      />
    ),
  },
  {
    id: "pages",
    tab: "general",
    section: () => "",
    title: () => tr("Sidebar pages"),
    description: () => tr("The pages the sidebar lists; a hidden one is still in the tray's menu and a click away here"),
    control: () => <SidebarPages />,
  },
  {
    id: "tray",
    tab: "general",
    section: () => "",
    title: () => tr("Keep running in the tray"),
    description: () => tr("Closing the window leaves the gateway serving your agents."),
    control: () => <OnOff testID="setting-tray" on={s()?.closeToTray !== false} onChange={(on) => void setSetting("closeToTray", on)} />,
  },
  {
    id: "tray-click",
    tab: "general",
    section: () => "",
    title: () => tr("Click the tray icon to"),
    control: () => (
      <Segs
        testID="setting-tray-click"
        value={s()?.trayClick ?? "panel"}
        onChange={(v) => void setSetting("trayClick", v)}
        options={[
          { value: "panel", label: tr("Open the quick panel") },
          { value: "window", label: tr("Open the main window") },
        ]}
      />
    ),
  },

  // Network and sharing
  {
    id: "proxy",
    tab: "network",
    section: () => "",
    title: () => tr("Proxy"),
    description: () => {
      const p = s()?.proxy ?? "";
      if (p === "") return tr("Auto: requests to vendors without a proxy of their own go through the system's (HTTPS_PROXY), else direct");
      if (p === "direct") return tr("Off: requests to vendors without a proxy of their own go direct, whatever the system says");
      return tr("Requests to vendors without a proxy of their own go through %s", p);
    },
    control: () => <ProxyControl />,
    below: () => (
      <If when={proxyEditing() || proxyMode() === "custom"}>
        <ProxyAddress />
      </If>
    ),
  },
  {
    id: "port",
    tab: "network",
    section: () => tr("Gateway"),
    title: () => tr("Gateway port"),
    description: () => tr("Agents connected before a change are connected again with the new port."),
    control: () => <PortControl />,
  },
  {
    id: "bare",
    tab: "network",
    section: () => tr("Gateway"),
    title: () => tr("Model ids without the provider"),
    description: () =>
      tr("The gateway's model list also names each model by its own id, as deepseek-chat beside deepseek/deepseek-chat, for apps moved from another proxy. One that several providers serve goes to them in the Providers order"),
    control: () => <OnOff testID="setting-bare" on={!!s()?.bareModelIds} onChange={(on) => void setSetting("bareModelIds", on)} />,
  },
  {
    id: "lan",
    tab: "network",
    section: () => tr("Local network"),
    title: () => tr("Share on the local network"),
    description: () => tr("Agents on other computers on this network, or through a tunnel to this one, can use pansy's models with the gateway key from Gateway"),
    control: () => <OnOff testID="setting-lan" on={!!s()?.lan} onChange={(on) => void setSetting("lan", on, true)} />,
  },
  {
    id: "origins",
    tab: "network",
    section: () => tr("Web pages"),
    title: () => tr("Web pages that may call pansy"),
    description: () => tr("Scripts on these origins can call the gateway from a browser, each request with the gateway key. A page from any other site is refused; pages on this computer are not"),
    below: () => <ValueList values={s()?.origins ?? []} onChange={(v) => void setSetting("origins", v)} placeholder="https://app.example.com" mono testID="setting-origins" />,
  },

  // Privacy
  {
    id: "sign-ins",
    tab: "privacy",
    section: () => tr("Subscriptions"),
    title: () => tr("Use the agents' sign-ins"),
    description: () =>
      tr("Codex's ChatGPT, Copilot's GitHub and Gemini CLI's Google sign-ins are read where those agents keep them, and their plans' models served as providers. Off, only the accounts signed in from pansy are used"),
    control: () => <OnOff testID="setting-sign-ins" on={s()?.agentSignIns !== false} onChange={(on) => void setSetting("agentSignIns", on)} />,
  },
  {
    id: "key-store",
    tab: "privacy",
    section: () => tr("Keys"),
    title: () => tr("Keep API keys in the system's key store"),
    description: () =>
      tr("The login keyring on Linux (it needs secret-tool), the Keychain on macOS. On, pansy's providers file says only where each key is; off, the keys are in the file, which only you can read"),
    control: () => <OnOff testID="setting-key-store" on={s()?.keyStore === "system"} onChange={(on) => void setSetting("keyStore", on ? "system" : "file")} />,
  },
  {
    id: "found-rules",
    tab: "privacy",
    section: () => tr("How it's found"),
    title: () => tr("Rules"),
    description: () =>
      m()?.secrets || m()?.personal
        ? tr("Keys, tokens, emails and numbers whose checksum adds up: patterns that are certain, checked on every request")
        : tr("Turn on Mask secrets or Mask personal data first"),
    control: () => <Badge label={m()?.secrets || m()?.personal ? tr("Always on") : tr("Off")} tone={m()?.secrets || m()?.personal ? "success" : "default"} />,
  },
  {
    id: "masked",
    tab: "privacy",
    section: () => tr("What to mask"),
    title: () => tr("What was masked"),
    description: () => tr("Each request masking changed, what the vendor saw, and how, on Insights › Masking"),
    control: () => <TextButton label={tr("Open")} onPress={() => go("masking")} testID="setting-open-masking" />,
  },
  {
    id: "secrets",
    tab: "privacy",
    section: () => tr("What to mask"),
    title: () => tr("Mask secrets"),
    description: () => tr("API keys, private keys, tokens and passwords go to vendors as placeholders, and come back as they were"),
    control: () => <OnOff testID="setting-mask-secrets" on={!!m()?.secrets} onChange={(on) => void setMasking({ secrets: on })} />,
  },
  {
    id: "personal",
    tab: "privacy",
    section: () => tr("What to mask"),
    title: () => tr("Mask personal data"),
    description: () => tr("Emails, phone numbers, ID and bank card numbers too"),
    control: () => <OnOff testID="setting-mask-personal" on={!!m()?.personal} onChange={(on) => void setMasking({ personal: on })} />,
  },
  ...MASK_CATEGORIES.map(
    (c): Setting => ({
      id: `category-${c.id}`,
      tab: "privacy",
      section: () => tr("What to mask"),
      title: () => `· ${categoryLabel(c.id).title}`,
      description: () => categoryLabel(c.id).description,
      when: () => !!m()?.personal,
      control: () => (
        <OnOff
          testID={`setting-mask-${c.id}`}
          on={m()?.kinds[c.id] ?? c.on}
          onChange={(on) => void setMasking({ kinds: { ...(m()?.kinds ?? {}), [c.id]: on } })}
        />
      ),
    }),
  ),
  {
    id: "words",
    tab: "privacy",
    section: () => tr("What to mask"),
    title: () => tr("Masked words"),
    description: () => tr("Your own words to keep from vendors, one on each row: names, codenames, hosts"),
    below: () => <ValueList values={m()?.words ?? []} onChange={(words) => void setMasking({ words })} placeholder={tr("names, codenames, hosts")} hide testID="setting-mask-words" />,
  },
  {
    id: "any-case",
    tab: "privacy",
    section: () => tr("What to mask"),
    title: () => tr("Masked words in any case"),
    description: () => tr("A word also matches in other capitals: Nightjar masks NIGHTJAR and nightjar too"),
    control: () => <OnOff testID="setting-mask-any-case" on={!!m()?.wordsAnyCase} onChange={(on) => void setMasking({ wordsAnyCase: on })} />,
  },
  {
    id: "whole",
    tab: "privacy",
    section: () => tr("What to mask"),
    title: () => tr("Whole words only"),
    description: () => tr("A word is not masked inside a longer one: ABC masks ABC and 'ABC' but not XABCD, and 123456 not 0.123456"),
    control: () => <OnOff testID="setting-mask-whole" on={!!m()?.wordsWhole} onChange={(on) => void setMasking({ wordsWhole: on })} />,
  },
  {
    id: "rules",
    tab: "privacy",
    section: () => tr("What to mask"),
    title: () => tr("Masking rules"),
    description: () => tr("Secrets pansy doesn't know, such as a gateway's own keys: what they start with, or a regular expression. Masked while Mask secrets is on"),
    below: () => <RuleList />,
  },
  {
    id: "alert",
    tab: "privacy",
    section: () => tr("When a value is found"),
    title: () => tr("Masking alert"),
    description: () => tr("A notification when masking swaps a value for the first time: its kind and how many, never the value itself"),
    control: () => <OnOff testID="setting-mask-alert" on={s()?.maskAlert !== false} onChange={(on) => void setSetting("maskAlert", on)} />,
  },
  {
    id: "everything",
    tab: "privacy",
    section: () => tr("When a value is found"),
    title: () => tr("Mask everything found"),
    description: () => tr("Turn on Mask secrets or Mask personal data first"),
    when: () => !m()?.secrets && !m()?.personal,
  },
  ...ActionRows(SECRET_KINDS, () => !!m()?.secrets),
  ...ActionRows(PERSONAL_KINDS, () => !!m()?.personal),
  {
    id: "allow",
    tab: "privacy",
    section: () => tr("When a value is found"),
    title: () => tr("Never mask"),
    description: () => tr("Values that always go as written, each whole and in any case, one on each row: a team address, a test key"),
    below: () => <ValueList values={m()?.allow ?? []} onChange={(allow) => void setMasking({ allow })} placeholder="team@example.com" testID="setting-mask-allow" />,
  },
  {
    id: "try",
    tab: "privacy",
    section: () => tr("Try it"),
    title: () => tr("Try it"),
    description: () => tr("Paste text an agent might send: it is masked here as a request would be, and kept nowhere"),
    below: () => <TryIt />,
  },

  // Diagnostics
  {
    id: "health",
    tab: "diagnostics",
    section: () => "",
    title: () => tr("Health check"),
    description: () => healthSaid(),
    control: () => <HealthControl />,
    below: () => <FindingList />,
  },
  {
    id: "test-request",
    tab: "diagnostics",
    section: () => "",
    title: () => tr("Send a test request"),
    description: () => testSaid(),
    control: () => <TestControl />,
    below: () => <TestResult />,
  },
  {
    id: "restore-all",
    tab: "diagnostics",
    section: () => tr("If pansy stops"),
    title: () => tr("Restore every agent for now"),
    description: () => tr("Each agent connected to pansy goes back to the config it had before, and works without the gateway. pansy remembers its model"),
    control: () => <RestoreAll />,
  },
  {
    id: "resume-all",
    tab: "diagnostics",
    section: () => tr("If pansy stops"),
    title: () => tr("Connect them again"),
    description: () => tr("The agents restored for now go back on pansy with the models they had"),
    control: () => <ResumeAll />,
  },
  {
    id: "terminal",
    tab: "diagnostics",
    section: () => tr("If pansy stops"),
    title: () => tr("From a terminal"),
    description: () => tr("When the window won't open, the pansy command does the same"),
    below: () => <FromATerminal />,
  },

  // About
  {
    id: "version",
    tab: "about",
    section: () => "",
    title: () => "Pansy",
    description: () => tr("Every agent, every provider's models."),
    control: () => <Text class="text-[12px] text-muted-foreground">{tr("Version %s", versionSaid(Settings.version()))}</Text>,
  },
  {
    id: "updates",
    tab: "about",
    section: () => "",
    title: () => tr("Updates"),
    description: () => updateSaid(),
    control: () => <UpdateControl />,
  },
  {
    id: "whats-new",
    tab: "about",
    section: () => "",
    title: () => tr("What's new"),
    description: () => (hasNotes() ? tr("What changed in this version, and in one that is waiting") : tr("No release notes yet: they come with an update")),
    control: () => <TextButton label={tr("Open")} disabled={!hasNotes()} testID="whats-new-open" onPress={openNotes} />,
  },
  {
    id: "folder",
    tab: "about",
    section: () => "",
    title: () => tr("Config folder"),
    description: () => Settings.home(),
    control: () => <TextButton label={tr("Show")} onPress={() => void Settings.openHome().catch((error) => toast(reason(error), "error"))} testID="setting-open-home" />,
  },
  {
    id: "address",
    tab: "about",
    section: () => "",
    title: () => tr("Gateway address"),
    description: () => gateway()?.url ?? "",
    control: () => <TextButton label={tr("Copy")} onPress={() => void Clipboard.writeText(gateway()?.url ?? "").then(() => toast(tr("Copied")))} />,
  },
];

/** Whether a setting's name or the line under it has `query`, in the window's language or in English. */
function matches(setting: Setting, query: string): boolean {
  const q = query.trim().toLowerCase();
  if (!q) return true;
  return [setting.title(), setting.description?.() ?? "", setting.section()].some((t) => t.toLowerCase().includes(q));
}

/** The settings shown: the open page's, or every page's that match the query; in cards by page and section. */
function cards(): { key: string; tab: SettingsTab; label: string; rows: Setting[] }[] {
  const query = settingsQuery().trim();
  const out: { key: string; tab: SettingsTab; label: string; rows: Setting[] }[] = [];
  for (const setting of SETTINGS) {
    if (query ? !matches(setting, query) : setting.tab !== settingsTab()) continue;
    if (setting.when && !setting.when()) continue;
    const key = `${setting.tab}:${setting.section()}`;
    const last = out[out.length - 1];
    if (last?.key === key) last.rows.push(setting);
    else out.push({ key, tab: setting.tab, label: setting.section(), rows: [setting] });
  }
  return out;
}

function SettingView(props: { setting: Setting; first: boolean }) {
  const setting = props.setting;
  // Made once: each follows the settings on its own.
  const [control, below, badge] = untracked(() => [setting.control?.(), setting.below?.(), setting.badge?.()]);
  return (
    <SettingRow first={props.first} testID={`setting-row-${setting.id}`} title={setting.title()} description={setting.description?.()} badge={badge} below={below}>
      {control}
    </SettingRow>
  );
}

/** Settings' cards; with the sidebar hidden, its pages and Find a setting head them (magpie's #setTabs). */
export function SettingsPage() {
  const tabName = (tab: SettingsTab) => SETTINGS_TABS().find((t) => t.id === tab)?.name ?? tab;
  return (
    <PageView testID="page-settings">
      <If when={!sideShown()}>
        <View class="flex-row items-center gap-3 pb-1">
          <Segs
            testID="settings-tabs"
            value={settingsQuery().trim() ? ("" as SettingsTab) : settingsTab()}
            onChange={(tab) => {
              settingsQuery.set("");
              settingsTab.set(tab);
            }}
            options={SETTINGS_TABS().map((t) => ({ value: t.id, label: t.name }))}
          />
          <View class="flex-1" />
          <Input value={settingsQuery()} onChange={(v) => settingsQuery.set(v)} placeholder={tr("Find a setting")} testID="settings-find" class="w-[220px]" />
        </View>
      </If>
      <Each of={cards()} key={(c) => c.key}>
        {(card, i) => (
          <View class="gap-[6px]">
            <If when={settingsQuery().trim() ? true : !!card().label}>
              <RowHead label={settingsQuery().trim() ? [tabName(card().tab), card().label].filter(Boolean).join(" › ") : card().label} class={i() === 0 ? "" : "pt-2"} />
            </If>
            <List>
              <Each of={card().rows} key={(r) => r.id}>
                {(r, j) => <SettingView setting={r()} first={j() === 0} />}
              </Each>
            </List>
          </View>
        )}
      </Each>
      <If when={cards().length === 0}>
        <View class="items-center py-10">
          <Text testID="settings-none" class="text-[12.5px] text-muted-foreground">
            {tr("No setting has “%s”", settingsQuery().trim())}
          </Text>
        </View>
      </If>
      <If when={!settingsQuery().trim() && settingsTab() === "privacy" && !maskingOn()}>
        <Text class="px-1 text-[11.5px] text-faint">{tr("Masking is off: requests go to vendors as they are.")}</Text>
      </If>
    </PageView>
  );
}

