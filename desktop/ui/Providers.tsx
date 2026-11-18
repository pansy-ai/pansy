// Providers, as magpie lists them: one card of rows (logo, name, host and
// models, the agents on it, its key, a switch) in the order they are tried
// in, which a right click on a row changes, and the ones switched off
// folded under them. Each opens its editor in a modal: its plan or region,
// its keys (tried in order, the next one taking over when one runs out),
// its server's address, its models, headers, key header and proxy, and its
// endpoints. "Add provider" opens the vendors under the list, by kind, a
// vendor's global and China endpoints as one; picking one asks for its key.
// A subscription (ChatGPT as Codex uses it, GitHub Copilot, Gemini Code
// Assist as Gemini CLI uses it) is a provider whose accounts are sign-ins:
// the agent's own, read where it keeps it, and those signed in from here, in
// the browser or by a code.
import { Clipboard, effect, openUrl, signal, Text, View } from "nsh";
import { ContextMenu, MenuItem, MenuSeparator } from "nsh/base";
import { Providers, Subscriptions } from "pansy/native";
import { tr, trCore, trn } from "./i18n";
import {
  act,
  agents,
  ask,
  choiceName,
  go,
  hostOf,
  importFound,
  isBusy,
  loadImportFound,
  loadProviders,
  presets,
  providerOf,
  providers,
  toast,
  type Account,
  type Endpoints,
  type ImportItem,
  type ImportSource,
  type Preset,
  type PresetKind,
  type Protocol,
  type Provider,
  type Region,
  type SignInState,
  type SubscriptionKind,
} from "./model";
import {
  agentLogo,
  Badge,
  Check,
  Code,
  CopyButton,
  Dot,
  Dropdown,
  FormRow,
  Hint,
  Icon,
  IconButton,
  Input,
  List,
  Logo,
  logoNamed,
  Modal,
  Mono,
  Page,
  PrimaryButton,
  Row,
  RowHead,
  Segs,
  SHADOW,
  Switch,
  TextButton,
  Who,
  type IconName,
} from "./parts";

const PROTOCOL_NAMES: Record<Protocol, string> = {
  anthropic: "Anthropic",
  chat: "OpenAI",
  responses: "Responses",
  gemini: "Gemini",
};

/** The vendors are open under the list. */
export const adding = signal(false);
/** The provider whose editor is open. */
const editing = signal<string | null>(null);
/** The preset whose key is being asked for, or the custom form. */
const chosen = signal<Preset | "custom" | null>(null);
/** The providers switched off are folded away under the list. */
const offFolded = signal(true);
/** What "Find a provider" holds. */
const finding = signal("");
/** The subscription being signed in to. */
const signingIn = signal<SubscriptionKind | null>(null);
/** The import from other apps is open; `only` picks just the configurations the page found. */
const importing = signal<{ only?: string[] } | null>(null);
/** The custom form as a pasted curl command or environment lines filled it. */
const customDraft = signal<CustomDraft | null>(null);
/** The import link waiting for a yes: one the system opened (pansy://) or one pasted. */
const linkOpen = signal<string | null>(null);
/** "Paste a configuration" is open. */
const pasting = signal(false);

/** A custom provider's form, filled in from a paste: `more` the other base URLs it gave, set once it is added. */
interface CustomDraft {
  name: string;
  protocol: Protocol;
  baseUrl: string;
  key: string;
  models: string;
  note: string;
  more: { protocol: Protocol; url: string }[];
}

/** What an import link adds, as pansy shows it before the yes (`import::link::view`): the key masked. */
interface LinkView {
  error?: string;
  id: string;
  name: string;
  icon: string;
  endpoints: { protocol: Protocol; url: string }[];
  hosts: string[];
  key: string;
  keyless: boolean;
  keyUrl: string;
  models: string[];
  replaces: string;
}

/** What a pasted text is read as (`Providers.readPaste`). */
interface Pasted {
  kind: "link" | "curl" | "env" | "";
  link?: LinkView;
  bases: { protocol: Protocol; url: string }[];
  key: string;
  placeholder: string;
  model: string;
  name: string;
  urlHole: string;
}

const readPaste = async (text: string) => JSON.parse(await Providers.readPaste(text)) as Pasted;

/** Text worth reading as a configuration: an import link, a curl command, the environment lines of an API. */
const looksPasted = (t: string) =>
  /^\s*(pansy|magpie):/i.test(t) || /^\s*curl(\.exe)?\s/i.test(t) || /\b(ANTHROPIC|OPENAI|GEMINI|GOOGLE)_[A-Z_]*(URL|BASE|ENDPOINT|KEY|TOKEN)\b/i.test(t);

/** Says what a paste filled in, and what is left to do, as magpie does. */
function pastedNote(c: Pasted): string {
  const took: string[] = [];
  for (const b of c.bases) took.push(tr("Base URL (%s)", PROTOCOL_NAMES[b.protocol]));
  if (c.key) took.push(tr("API key"));
  if (c.model) took.push(tr("model %s", c.model));
  const notes: string[] = [];
  if (took.length) notes.push(c.kind === "env" ? tr("Filled in from the pasted text: %s.", took.join(" · ")) : tr("Filled in from the curl command: %s.", took.join(" · ")));
  if (c.kind === "curl" && !c.bases.length) notes.push(tr("Its URL isn't an address ending in /chat/completions, /messages or /responses: type the base URL."));
  if (c.urlHole) notes.push(tr("Replace %s in the base URL.", c.urlHole));
  if (c.placeholder) notes.push(tr("Its key, %s, only stands for one: paste your own API key.", c.placeholder));
  return notes.join(" ");
}

/** The custom form a curl command or environment lines fill. */
function draftOf(c: Pasted): CustomDraft {
  const [first, ...more] = c.bases;
  return { name: c.name, protocol: first?.protocol ?? "chat", baseUrl: first?.url ?? "", key: c.key, models: c.model, note: pastedNote(c), more };
}

/**
 * Opens what a paste is: an import link's confirmation, or the custom form
 * filled in from a curl command or environment lines with a base URL; false
 * for text that is neither.
 */
async function openPasted(text: string): Promise<boolean> {
  const c = await readPaste(text);
  if (c.kind === "link") {
    linkOpen.set(text.trim());
    return true;
  }
  if ((c.kind === "curl" || c.kind === "env") && c.bases.length) {
    customDraft.set(draftOf(c));
    chosen.set("custom");
    return true;
  }
  return false;
}

/** A pansy:// link the system opened the app with: the Providers page, asking before it adds anything. */
export function openLink(url: string) {
  go("providers");
  linkOpen.set(url);
}

/** The subscriptions pansy signs in to, as the add sheet lists them. */
const SUBSCRIPTIONS = (): { kind: SubscriptionKind; name: string; vendor: string; icon: string; note: string }[] => [
  { kind: "codex", name: "ChatGPT", vendor: "ChatGPT", icon: "codex-color", note: tr("Plus, Pro or Business, as Codex uses it") },
  { kind: "copilot", name: "GitHub Copilot", vendor: "GitHub", icon: "githubcopilot", note: tr("OpenAI's, Anthropic's and Google's models on your Copilot plan") },
  { kind: "gemini-cli", name: "Gemini Code Assist", vendor: "Google", icon: "geminicli-color", note: tr("Google's models on your Code Assist plan, as Gemini CLI uses them") },
];
const subscriptionOf = (kind: SubscriptionKind) => SUBSCRIPTIONS().find((x) => x.kind === kind)!;
/** The agent whose own sign-in a subscription reads. */
const AGENT_OF: Record<SubscriptionKind, string> = { codex: "Codex", copilot: "Copilot", "gemini-cli": "Gemini CLI" };

const usersOf = (p: Provider) => agents().filter((a) => a.connected && (providerOf(a.model)?.id === p.id || providerOf(a.smallModel)?.id === p.id));

const presetOf = (id: string | null) => presets().find((p) => p.id === id);
/** A vendor's China preset (`moonshot-cn`) beside its global one. */
const chinaOf = (p: Preset) => presets().find((x) => x.id === `${p.id}-cn`);
/** The global preset a China one is beside. */
const globalOf = (p: Preset) => (p.id.endsWith("-cn") ? presets().find((x) => x.id === p.id.slice(0, -3)) : undefined);
/** The first provider made from a preset. */
const providerFrom = (p: Preset) => providers().find((x) => x.preset === p.id);
/** A preset's name without what its brackets say ("Kimi (China)" → "Kimi"), for a row that stands for both. */
const shortName = (name: string) => name.replace(/\s*[(（][^()（）]*[)）]\s*$/, "") || name;
/** `http://localhost:11434/v1` → `http://localhost:11434`. */
const originOf = (url: string | undefined) => url?.match(/^[a-z]+:\/\/[^/?#]+/i)?.[0] ?? "";
/** Where a list of endpoints is, by its chat one first. */
const hostOfAll = (e: Endpoints) => hostOf((e.find((x) => x.protocol === "chat") ?? e[0])?.baseUrl);

/** A plan or region picker: segments for a few, a menu for many (Bedrock's regions). */
function RegionPicker(props: { regions: Region[]; value: string | null; onChange: (id: string) => void; testID: string }) {
  const options = () => props.regions.map((r) => ({ value: r.id, label: trCore(r.name) }));
  const value = () => props.value ?? "";
  return (
    <If when={props.regions.length <= 4} else={<Dropdown testID={props.testID} options={options()} value={value()} onChange={props.onChange} width={220} />}>
      <Segs testID={props.testID} options={options()} value={value()} onChange={props.onChange} />
    </If>
  );
}

/** A provider's endpoints, one to a line. */
function EndpointList(props: { endpoints: Endpoints }) {
  return (
    <Each of={props.endpoints} key={(e) => e.protocol}>
      {(e) => (
        <View class="h-[22px] flex-row items-center gap-3">
          <Text class="w-[64px] text-[11.5px] text-muted-foreground">{PROTOCOL_NAMES[e().protocol]}</Text>
          <Mono text={e().baseUrl} class="flex-1 line-clamp-1" />
        </View>
      )}
    </Each>
  );
}

/** The key's pill: the key, how many there are, none needed, or one wanted. */
function KeyState(props: { provider: Provider }) {
  const p = () => props.provider;
  const on = () => p().accounts.filter((a) => a.enabled);
  return (
    <View class="w-[120px] flex-row items-center gap-[6px]">
      <If
        when={on().length > 0}
        else={
          <If when={p().keyless} else={<><Dot tone="warn" /><Text class="text-[12px] text-warning">{tr("needs a key")}</Text></>}>
            <Hint text={p().kind === "local" ? tr("Local servers need no key") : tr("optional: free models need no key")}>
              <View class="flex-row items-center gap-[6px]">
                <Dot tone={p().enabled ? "on" : "off"} />
                <Text class="text-[12px] text-muted-foreground">{tr("no key")}</Text>
              </View>
            </Hint>
          </If>
        }
      >
        <Dot tone={p().enabled ? "on" : "off"} />
        <If when={on().length === 1} else={<Text class="text-[12px] text-muted-foreground">{tr("%d keys", on().length)}</Text>}>
          <Mono text={on()[0]?.masked ?? ""} class="text-[11.5px] text-muted-foreground" />
        </If>
      </If>
    </View>
  );
}

/** A subscription's pill: the account signed in, how many there are, or none switched on. */
function AccountState(props: { provider: Provider }) {
  const on = () => props.provider.accounts.filter((a) => a.enabled);
  return (
    <View class="w-[120px] flex-row items-center gap-[6px]">
      <If when={on().length > 0} else={<><Dot tone="warn" /><Text class="text-[12px] text-warning">{tr("no account on")}</Text></>}>
        <Dot tone={props.provider.enabled ? "on" : "off"} />
        <If when={on().length === 1} else={<Text class="text-[12px] text-muted-foreground">{tr("%d accounts", on().length)}</Text>}>
          <Text class="min-w-0 shrink text-[12px] text-muted-foreground line-clamp-1">{on()[0]?.display ?? ""}</Text>
        </If>
      </If>
    </View>
  );
}

function Item(props: { icon: IconName; label: string; onSelect: () => void; disabled?: boolean; testID?: string }) {
  return (
    <MenuItem
      disabled={props.disabled}
      testID={props.testID}
      onSelect={props.onSelect}
      class="h-[28px] flex-row items-center gap-[9px] rounded-[7px] px-2 hover:bg-pill focus:bg-pill disabled:opacity-40"
    >
      <Icon name={props.icon} color="fg-2" size={13} />
      <Text class="text-[12.5px] text-foreground">{props.label}</Text>
    </MenuItem>
  );
}

/** Puts a provider at `to` among all of them; the list shows it there at once. */
async function arrange(id: string, to: number) {
  const all = providers();
  const from = all.findIndex((p) => p.id === id);
  if (from < 0 || to < 0 || to >= all.length || to === from) return;
  const next = [...all];
  next.splice(to, 0, ...next.splice(from, 1));
  providers.set(next);
  if (await act("arrange", () => Providers.moveTo(id, to), tr("Provider order saved"))) return;
  await loadProviders();
}

/** A row's right click: where it is tried among the providers switched on. */
function ProviderMenu(props: { provider: Provider }) {
  const p = () => props.provider;
  const on = () => providers().filter((x) => x.enabled);
  const i = () => on().findIndex((x) => x.id === p().id);
  /** The place among all providers of the one switched on at `k`. */
  const at = (k: number) => providers().findIndex((x) => x.id === on()[k]?.id);
  return (
    <>
      <Item icon="top" label={tr("Move to top")} disabled={i() <= 0} testID={`provider-top-${p().id}`} onSelect={() => void arrange(p().id, at(0))} />
      <Item icon="up" label={tr("Move up")} disabled={i() <= 0} testID={`provider-up-${p().id}`} onSelect={() => void arrange(p().id, at(i() - 1))} />
      <Item icon="down" label={tr("Move down")} disabled={i() >= on().length - 1} testID={`provider-down-${p().id}`} onSelect={() => void arrange(p().id, at(i() + 1))} />
      <Item icon="bottom" label={tr("Move to bottom")} disabled={i() >= on().length - 1} testID={`provider-bottom-${p().id}`} onSelect={() => void arrange(p().id, at(on().length - 1))} />
      <MenuSeparator class="mx-1 my-1 h-px bg-line-2" />
      <Item icon="power" label={tr("Switch off")} testID={`provider-off-${p().id}`} onSelect={() => void act("provider", () => Providers.setEnabled(p().id, false)).then(() => loadProviders())} />
    </>
  );
}

/** The models of `p` a search's words name that its row doesn't say. */
function modelHits(p: Provider, words: string[]): string[] {
  const squash = (s: string) => s.toLowerCase().replace(/[\s._-]+/g, "");
  const own = [p.name, p.id, p.preset ?? "", ...p.endpoints.map((e) => e.baseUrl)].join(" ").toLowerCase();
  const rest = words.filter((w) => !own.includes(w));
  if (!rest.length) return [];
  return p.models.filter((m) => !m.hidden && rest.every((w) => `${squash(m.id)}|${squash(m.name)}`.includes(squash(w)))).map((m) => m.id);
}

/** Whether `p` says every word of the search, or serves a model that does. */
function matches(p: Provider, words: string[]): boolean {
  const own = [p.name, p.id, p.preset ?? "", ...p.endpoints.map((e) => e.baseUrl), ...p.accounts.map((a) => a.label)].join(" ").toLowerCase();
  return words.every((w) => own.includes(w)) || modelHits(p, words).length > 0;
}

function ProviderRow(props: { provider: Provider; first: boolean }) {
  const p = () => props.provider;
  const words = () => finding().trim().toLowerCase().split(/\s+/).filter(Boolean);
  const hits = () => modelHits(p(), words());
  const row = () => (
    <Row first={props.first} testID={`provider-${p().id}`} class="min-h-[49px]" onPress={() => editing.set(p().id)}>
      <Hint text={p().enabled ? tr("Right-click to move it: a model several providers serve goes to the higher one first") : p().name}>
        <View testID={`provider-open-${p().id}`} class="h-7 w-7 items-center justify-center" style={{ opacity: p().enabled ? 1 : 0.55 }}>
          <Logo name={logoNamed(p().icon)} size={22} />
        </View>
      </Hint>
      <View class="min-w-0 flex-1 gap-[1px]">
        <Who name={p().name} sub={trn(p().models.length, "%s · %d model", "%s · %d models", hostOfAll(p().endpoints), p().models.length)}>
          <If when={!p().enabled}>
            <Badge label={tr("Switched off")} />
          </If>
        </Who>
        <If when={hits().length > 0}>
          <Text class="text-[11.5px] text-primary line-clamp-1">{tr("Serves %s", hits().join(", "))}</Text>
        </If>
      </View>
      <View class="flex-row items-center gap-1">
        <Each of={usersOf(p())} key={(a) => a.id}>
          {(a) => (
            <Hint text={`${a().name} · ${choiceName(a().model)}`}>
              <View class="h-[22px] w-[26px] items-center justify-center rounded-[6px] bg-pill">
                <Logo name={agentLogo(a().id)} size={14} />
              </View>
            </Hint>
          )}
        </Each>
      </View>
      <If when={p().subscription} else={<KeyState provider={p()} />}>
        <AccountState provider={p()} />
      </If>
      <Switch
        checked={p().enabled}
        label={tr("Use %s", p().name)}
        testID={`provider-enabled-${p().id}`}
        onChange={(on) => void act("provider", () => Providers.setEnabled(p().id, on)).then(() => loadProviders())}
      />
      <Icon name="chev-right" color="faint" size={11} />
    </Row>
  );
  return (
    <If when={p().enabled} else={row()}>
      <ContextMenu trigger={row()} popupClass="min-w-[200px] rounded-[10px] border border-border bg-popover p-1" popupStyle={{ boxShadow: SHADOW.pop() }}>
        <ProviderMenu provider={p()} />
      </ContextMenu>
    </If>
  );
}

/** A subscription of the add sheet: a click signs in to an account of it. */
function SubscriptionTile(props: { kind: SubscriptionKind }) {
  const sub = () => subscriptionOf(props.kind);
  const added = () => providers().some((p) => p.subscription === props.kind);
  return (
    <View style={{ width: "33.33%" }} class="pr-2">
      <Hint text={added() ? tr("Signed in · click to add another account") : sub().note}>
        <View role="button" testID={`subscription-${props.kind}`} onPress={() => signingIn.set(props.kind)} class="h-[32px] flex-row items-center gap-2 overflow-hidden rounded-[7px] px-[6px] hover:bg-pill">
          <Logo name={logoNamed(sub().icon)} size={16} />
          <Text class="min-w-0 shrink text-[13px] text-foreground line-clamp-1">{sub().name}</Text>
          <If when={added()}>
            <Dot tone="on" />
          </If>
        </View>
      </Hint>
    </View>
  );
}

/** One vendor of the add sheet, or a vendor's global and China presets as one. */
function PresetTile(props: { preset: Preset; china?: Preset }) {
  const both = () => [props.preset, ...(props.china ? [props.china] : [])];
  const added = () => both().filter((x) => providerFrom(x));
  const name = () => (props.china ? shortName(props.preset.short || props.preset.name) : props.preset.short || props.preset.name);
  const title = () => {
    if (props.china) return both().map((x) => `${x === props.preset ? tr("Global") : tr("China")} ${x.host}${providerFrom(x) ? ` · ${tr("Added")}` : ""}`).join("\n");
    if (added().length) return tr("%s is already added — open it", props.preset.name);
    return `${props.preset.short ? `${props.preset.name} · ` : ""}${props.preset.note ? trCore(props.preset.note) : props.preset.host}`;
  };
  const open = () => {
    const next = both().find((x) => !providerFrom(x));
    if (next) chosen.set(next);
    else editing.set(providerFrom(props.preset)?.id ?? null);
  };
  return (
    <View style={{ width: "33.33%" }} class="pr-2">
      <Hint text={title()}>
        <View role="button" testID={`preset-${props.preset.id}`} onPress={open} class="h-[32px] flex-row items-center gap-2 overflow-hidden rounded-[7px] px-[6px] hover:bg-pill">
          <Logo name={logoNamed(props.preset.icon)} size={16} />
          <Text class="min-w-0 shrink text-[13px] text-foreground line-clamp-1">{name()}</Text>
          <If when={added().length > 0}>
            <Dot tone="on" />
          </If>
        </View>
      </Hint>
    </View>
  );
}

/** The vendors to add, by kind, under the list. */
function AddSection() {
  const query = signal("");
  const hit = (p: Preset) => {
    const q = query().trim().toLowerCase();
    return !q || p.name.toLowerCase().includes(q) || p.id.includes(q) || p.host.includes(q) || p.note.toLowerCase().includes(q) || trCore(p.note).toLowerCase().includes(q);
  };
  /** A kind's rows: a vendor's China preset with its global one. */
  const rows = (kind: PresetKind) =>
    presets()
      .filter((p) => p.kind === kind && !globalOf(p))
      .map((p) => ({ preset: p, china: chinaOf(p) }))
      .filter((r) => hit(r.preset) || (r.china && hit(r.china)));
  const subs = () => {
    const q = query().trim().toLowerCase();
    return SUBSCRIPTIONS().filter((x) => !q || x.name.toLowerCase().includes(q) || x.kind.includes(q) || x.vendor.toLowerCase().includes(q));
  };
  // an import link, a curl command or environment lines pasted here open
  // what they describe, as magpie's search box does
  const find = (v: string) => {
    query.set(v);
    if (!looksPasted(v)) return;
    void openPasted(v)
      .then((opened) => {
        if (opened && query() === v) query.set("");
      })
      .catch(() => {});
  };
  const kinds = (): { kind: PresetKind; title: string; note: string }[] => [
    { kind: "vendor", title: tr("Vendors"), note: tr("the makers' own APIs") },
    { kind: "relay", title: tr("Relays"), note: tr("one key, many vendors") },
    { kind: "local", title: tr("On this machine"), note: "" },
  ];
  return (
    <View testID="add-provider" class="gap-[6px] pt-[6px]">
      <RowHead label={providers().length ? tr("Add a provider") : tr("Add your first provider")}>
        <Input value={query()} onChange={find} autoFocus testID="preset-search" placeholder={tr("Find a vendor…")} class="h-[24px] w-[160px]" />
        <Hint text={tr("any OpenAI or Anthropic compatible URL")}>
          <TextButton label={tr("Custom provider")} icon="add" tone="accent" testID="add-custom" onPress={() => chosen.set("custom")} />
        </Hint>
        <Hint text={tr("Bring over providers set up in other apps")}>
          <TextButton label={tr("Import…")} testID="add-import" onPress={() => importing.set({})} />
        </Hint>
        <Hint text={tr("A curl command, environment lines or an import link from a vendor's page")}>
          <TextButton label={tr("Paste…")} testID="add-paste" onPress={() => pasting.set(true)} />
        </Hint>
        <If when={providers().length > 0}>
          <TextButton label={tr("Close")} testID="add-close" onPress={() => adding.set(false)} />
        </If>
      </RowHead>
      <If when={subs().length > 0}>
        <View testID="add-subscriptions" class="gap-1 pb-2">
          <View class="flex-row items-baseline gap-2 px-[6px] pb-1 pt-2">
            <Text class="text-[13px] font-semibold text-foreground">{tr("Subscriptions")}</Text>
            <Text class="text-[11.5px] text-faint">{tr("sign in, no key")}</Text>
          </View>
          <View class="flex-row flex-wrap">
            <Each of={subs()} key={(x) => x.kind}>
              {(x) => <SubscriptionTile kind={x().kind} />}
            </Each>
          </View>
        </View>
      </If>
      <Each of={kinds().filter((k) => rows(k.kind).length > 0)} key={(k) => k.kind}>
        {(k) => (
          <View class="gap-1 pb-2">
            <View class="flex-row items-baseline gap-2 px-[6px] pb-1 pt-2">
              <Text class="text-[13px] font-semibold text-foreground">{k().title}</Text>
              <Text class="text-[11.5px] text-faint">{k().note}</Text>
            </View>
            <View class="flex-row flex-wrap">
              <Each of={rows(k().kind)} key={(r) => r.preset.id}>
                {(r) => <PresetTile preset={r().preset} china={r().china} />}
              </Each>
            </View>
          </View>
        )}
      </Each>
      <If when={subs().length === 0 && kinds().every((k) => rows(k.kind).length === 0)}>
        <View testID="preset-none" class="flex-row flex-wrap items-center px-[6px] py-2">
          <Text class="text-[12.5px] text-muted-foreground">{tr("Nothing called “%s”.", query().trim())} </Text>
          <TextButton label={tr("Add it as a custom provider")} tone="link" onPress={() => chosen.set("custom")} />
        </View>
      </If>
    </View>
  );
}

/** Where a preset's provider is when it is the user's own: an Azure resource's endpoint, or a server's address. */
function PlaceField(props: { preset: Preset; value: string; onChange: (v: string) => void; onSubmit?: () => void; testID: string }) {
  const own = () => !!props.preset.endpoint;
  return (
    <FormRow
      label={own() ? tr("Endpoint") : tr("Server address")}
      hint={own() ? trCore(props.preset.endpointHint) : tr("Where the server listens; change the port, or give another computer's address")}
    >
      <Input
        value={props.value}
        onChange={props.onChange}
        onSubmit={props.onSubmit}
        mono
        testID={props.testID}
        placeholder={own() ? props.preset.endpoint : originOf(props.preset.endpoints[0]?.baseUrl)}
      />
    </FormRow>
  );
}

/** A preset's key, asked for in a modal, with its plan or region and where it is. */
function AddPreset(props: { preset: Preset }) {
  /** The preset picked: a vendor's global or China one. */
  const pick = signal(props.preset);
  const p = () => pick();
  const pair = () => {
    const global = globalOf(props.preset) ?? props.preset;
    const china = chinaOf(global);
    return china ? [global, china] : null;
  };
  const region = signal<string | null>(props.preset.regions[0]?.id ?? null);
  const at = () => p().regions.find((r) => r.id === region()) ?? null;
  const key = signal("");
  const place = signal("");
  const keyUrl = () => at()?.keyUrl || p().keyUrl;
  const website = () => at()?.website || p().website;
  const placed = () => !!p().endpoint || p().kind === "local";
  const ready = () => (p().keyless || !!key().trim()) && (!p().endpoint || !!place().trim());
  const close = () => chosen.set(null);
  const add = () => {
    if (!ready()) return;
    void act("add", () => Providers.add(p().id, key().trim(), "", region() ?? "", place().trim()), tr("%s added", p().name)).then((ok) => {
      if (!ok) return;
      close();
      adding.set(false);
      void loadProviders();
      // Fill in the vendor's own model list.
      void Providers.refreshModels(p().id)
        .then(() => loadProviders())
        .catch(() => {});
    });
  };
  return (
    <Modal
      testID="preset-modal"
      title={p().name}
      icon={<Logo name={logoNamed(p().icon)} size={18} />}
      aside={
        <View class="flex-row items-center gap-2">
          <If when={p().note}>
            <Text class="text-[11.5px] text-muted-foreground line-clamp-1">{trCore(p().note)}</Text>
          </If>
          <If when={website()}>
            <TextButton label={`${hostOf(website())} ↗`} tone="link" onPress={() => void openUrl(website())} />
          </If>
        </View>
      }
      onClose={close}
      foot={
        <>
          <View class="flex-1" />
          <TextButton label={tr("Cancel")} onPress={close} />
          <PrimaryButton label={tr("Add")} testID="preset-add" disabled={!ready() || isBusy("add")} onPress={add} />
        </>
      }
    >
      <If when={pair()}>
        <FormRow label={tr("Region")}>
          <Segs
            testID="preset-area"
            value={p().id}
            onChange={(id) => {
              const next = pair()?.find((x) => x.id === id);
              if (!next) return;
              pick.set(next);
              region.set(next.regions[0]?.id ?? null);
            }}
            options={(pair() ?? []).map((x, i) => ({ value: x.id, label: i ? tr("China") : tr("Global") }))}
          />
        </FormRow>
      </If>
      <If when={p().regions.length > 0}>
        <FormRow label={trCore(p().regionLabel)} hint={tr("which endpoint %s is reached through", p().name)}>
          <RegionPicker testID="preset-region" regions={p().regions} value={region()} onChange={(id) => region.set(id)} />
        </FormRow>
      </If>
      <If when={placed()}>
        <PlaceField preset={p()} value={place()} onChange={(v) => place.set(v)} onSubmit={add} testID="preset-place" />
      </If>
      <FormRow label={tr("API key")} hint={tr("Kept in pansy's providers.json, readable by you alone. Nothing is read from your shell.")}>
        <View class="flex-row items-center gap-2">
          <Input
            value={key()}
            onChange={(v) => key.set(v)}
            onSubmit={add}
            masked
            mono
            autoFocus={!placed()}
            testID="preset-key"
            placeholder={p().keyHint ? trCore(p().keyHint) : p().keyless ? tr("optional for local servers") : tr("paste an API key")}
            class="flex-1"
          />
          <If when={keyUrl()}>
            <TextButton label={tr("Get a key ↗")} tone="link" onPress={() => void openUrl(keyUrl())} />
          </If>
        </View>
      </FormRow>
      <If when={p().endpoint}>
        <FormRow label={tr("Models")}>
          <Text class="text-[12px] text-muted-foreground">{tr("No need to type them: once added, pansy lists the deployments this key can call. Type a deployment's name as a model id only if that list can't be fetched.")}</Text>
        </FormRow>
      </If>
      <If when={!p().endpoint}>
        <FormRow label={tr("Endpoints")} top>
          <EndpointList endpoints={at()?.endpoints ?? p().endpoints} />
        </FormRow>
      </If>
    </Modal>
  );
}

/** Any OpenAI, Anthropic or Gemini compatible URL. */
function AddCustom() {
  const draft = customDraft();
  const form = signal({ name: draft?.name ?? "", protocol: draft?.protocol ?? ("chat" as Protocol), baseUrl: draft?.baseUrl ?? "", key: draft?.key ?? "", models: draft?.models ?? "" });
  /** What a paste filled in, and what is left to do. */
  const note = signal(draft?.note ?? "");
  /** The other base URLs a paste gave, set once it is added. */
  let more = draft?.more ?? [];
  const set = (patch: Partial<ReturnType<typeof form>>) => form.update((f) => ({ ...f, ...patch }));
  const close = () => {
    chosen.set(null);
    customDraft.set(null);
  };
  // a curl command or environment lines pasted into the URL or the key fill
  // the form (@mark_li21 on X); an import link opens its confirmation
  const typed = (field: "baseUrl" | "key", v: string) => {
    set({ [field]: v });
    if (!looksPasted(v)) return;
    void readPaste(v)
      .then((c) => {
        if (form()[field] !== v) return;
        if (c.kind === "link") {
          close();
          linkOpen.set(v.trim());
          return;
        }
        if (c.kind !== "curl" && c.kind !== "env") return;
        const d = draftOf(c);
        const f = form();
        set({
          name: f.name.trim() || d.name,
          protocol: d.baseUrl ? d.protocol : f.protocol,
          baseUrl: d.baseUrl || (field === "baseUrl" ? "" : f.baseUrl),
          key: d.key || (field === "key" ? "" : f.key),
          models: [f.models.trim(), d.models].filter(Boolean).join(", "),
        });
        more = d.more;
        note.set(d.note);
      })
      .catch(() => {});
  };
  const add = () => {
    const f = form();
    void act("add", async () => {
      const id = await Providers.addCustom(f.name, f.protocol, f.baseUrl, f.key, f.models);
      for (const b of more.filter((b) => b.protocol !== f.protocol)) await Providers.setEndpoint(id, b.protocol, b.url);
    }, tr("%s added", f.name)).then((ok) => {
      if (!ok) return;
      close();
      adding.set(false);
      void loadProviders();
    });
  };
  return (
    <Modal
      testID="custom-modal"
      title={tr("Custom provider")}
      icon={<Logo name="generic" size={18} />}
      onClose={close}
      foot={
        <>
          <View class="flex-1" />
          <TextButton label={tr("Cancel")} onPress={close} />
          <PrimaryButton label={tr("Add")} testID="custom-add" disabled={!form().name.trim() || !form().baseUrl.trim() || isBusy("add")} onPress={add} />
        </>
      }
    >
      <If when={note()}>
        <Text testID="custom-note" class="rounded-[8px] bg-accent px-3 py-2 text-[12px] leading-[18px] text-accent-foreground">
          {note()}
        </Text>
      </If>
      <FormRow label={tr("Name")}>
        <Input value={form().name} onChange={(v) => set({ name: v })} autoFocus testID="custom-name" placeholder={tr("My relay")} />
      </FormRow>
      <FormRow label={tr("Protocol")}>
        <Segs
          testID="custom-protocol"
          value={form().protocol}
          onChange={(v) => set({ protocol: v })}
          options={(["chat", "responses", "anthropic", "gemini"] as Protocol[]).map((p) => ({ value: p, label: PROTOCOL_NAMES[p] }))}
        />
      </FormRow>
      <FormRow label={tr("Base URL")} hint={tr("OpenAI: up to /v1. Anthropic: before /v1/messages. Gemini: before /v1beta.")}>
        <Input value={form().baseUrl} onChange={(v) => typed("baseUrl", v)} mono testID="custom-url" placeholder="https://api.example.com/v1" />
      </FormRow>
      <FormRow label={tr("API key")}>
        <Input value={form().key} onChange={(v) => typed("key", v)} masked mono testID="custom-key" placeholder={tr("paste an API key")} />
      </FormRow>
      <FormRow label={tr("Models")} hint={tr("Separate with commas; Fetch models gets the rest when the vendor lists them.")}>
        <Input value={form().models} onChange={(v) => set({ models: v })} mono testID="custom-models" placeholder="model-a, model-b" />
      </FormRow>
    </Modal>
  );
}

function Keys(props: { provider: Provider }) {
  const p = () => props.provider;
  const newKey = signal("");
  /** The key whose proxy is being set. */
  const proxyOf = signal<string | null>(null);
  const add = () => {
    const key = newKey().trim();
    if (!key) return;
    void act(`key:${p().id}`, () => Providers.addKey(p().id, key, ""), tr("Key added")).then((ok) => {
      if (ok) newKey.set("");
      void loadProviders();
    });
  };
  return (
    <List>
      <Each of={p().accounts} key={(a) => a.id}>
        {(a, i) => (
          <Row first={i() === 0} testID={`key-${p().id}-${a().id}`} class="min-h-[36px] py-[4px] pl-[10px] pr-[6px]">
            <Mono text={a().masked} class={a().enabled ? "" : "text-faint"} />
            <Text class="flex-1 text-[11.5px] text-muted-foreground line-clamp-1">{a().display}</Text>
            <IconButton
              icon="globe"
              label={a().proxy ? tr("This key's proxy: %s", a().proxy) : tr("A proxy for this key")}
              testID={`key-proxy-open-${p().id}-${i()}`}
              onPress={() => proxyOf.set(proxyOf() === a().id ? null : a().id)}
            />
            <IconButton icon="down" label={tr("Move down")} onPress={() => void act("key", () => Providers.moveKey(p().id, a().id, 1)).then(() => loadProviders())} />
            <Switch
              checked={a().enabled}
              label={tr("Use this key")}
              testID={`key-enabled-${p().id}-${a().id}`}
              onChange={(on) => void act("key", () => Providers.setKeyEnabled(p().id, a().id, on)).then(() => loadProviders())}
            />
            <IconButton
              icon="trash-2"
              label={tr("Remove key")}
              testID={`key-remove-${p().id}-${a().id}`}
              onPress={async () => {
                if (await ask({ title: tr("Remove this key?"), description: a().masked, confirm: tr("Remove"), danger: true })) {
                  await act("key", () => Providers.removeKey(p().id, a().id));
                  await loadProviders();
                }
              }}
            />
          </Row>
        )}
      </Each>
      <If when={p().accounts.some((a) => a.id === proxyOf())}>
        <View class="gap-1 border-t border-line-2 px-[10px] py-2">
          <Text class="text-[11.5px] text-muted-foreground">
            {tr("A proxy for %s alone, over the provider's: an HTTP or SOCKS address, direct for none, empty for the provider's.", p().accounts.find((a) => a.id === proxyOf())?.masked ?? "")}
          </Text>
          <SavedInput
            value={p().accounts.find((a) => a.id === proxyOf())?.proxy ?? ""}
            placeholder="socks5://127.0.0.1:1080"
            mono
            testID={`key-proxy-${p().id}`}
            onSave={(proxy) => act("key", () => Providers.setKeyProxy(p().id, proxyOf() ?? "", proxy), tr("Proxy saved")).then(() => loadProviders())}
          />
        </View>
      </If>
      <Row first={p().accounts.length === 0} class="min-h-[38px] py-[6px] pl-[10px] pr-[6px]">
        <Icon name="add" color="muted-foreground" size={12} />
        <Input
          value={newKey()}
          onChange={(v) => newKey.set(v)}
          onSubmit={add}
          masked
          mono
          testID={`key-new-${p().id}`}
          placeholder={p().accounts.length ? tr("Add another key") : p().keyless ? tr("optional for local servers") : tr("paste an API key")}
          class="flex-1 border-transparent"
        />
        <TextButton label={tr("Add key")} testID={`key-add-${p().id}`} disabled={!newKey().trim() || isBusy(`key:${p().id}`)} onPress={add} />
      </Row>
    </List>
  );
}

/** A subscription's accounts, tried in order: who, on which plan, kept where; one signed in from pansy can be signed out. */
function SignIns(props: { provider: Provider }) {
  const p = () => props.provider;
  const kind = () => p().subscription!;
  const where = (a: Account) => (a.signIn?.source === "agent" ? tr("%s's own sign-in", AGENT_OF[kind()]) : tr("Signed in from pansy"));
  return (
    <List>
      <Each of={p().accounts} key={(a) => a.id}>
        {(a, i) => (
          <Row first={i() === 0} testID={`account-${p().id}-${a().id}`} class="min-h-[40px] py-[4px] pl-[10px] pr-[6px]">
            <View class="min-w-0 flex-1 gap-[1px]">
              <Text class={`text-[12.5px] line-clamp-1 ${a().enabled ? "text-foreground" : "text-faint"}`}>{a().display}</Text>
              <Text class="text-[11px] text-muted-foreground line-clamp-1">{[a().signIn?.plan, where(a())].filter(Boolean).join(" · ")}</Text>
            </View>
            <If when={kind() === "gemini-cli"}>
              <Hint text={tr("The Google Cloud project it bills: Code Assist Standard and Enterprise ask for one; empty for the one Google gives it")}>
                <View class="w-[190px]">
                  <SavedInput
                    value={a().signIn?.project ?? ""}
                    mono
                    placeholder={tr("Cloud project")}
                    testID={`account-project-${p().id}-${a().id}`}
                    onSave={(project) => act("key", () => Subscriptions.setProject(kind(), a().id, project), tr("Project saved")).then(() => loadProviders())}
                  />
                </View>
              </Hint>
            </If>
            <IconButton icon="down" label={tr("Move down")} onPress={() => void act("key", () => Providers.moveKey(p().id, a().id, 1)).then(() => loadProviders())} />
            <Switch
              checked={a().enabled}
              label={tr("Use this account")}
              testID={`account-enabled-${p().id}-${a().id}`}
              onChange={(on) => void act("key", () => Providers.setKeyEnabled(p().id, a().id, on)).then(() => loadProviders())}
            />
            <If when={a().signIn?.source === "pansy"} else={<View class="w-[26px]" />}>
              <IconButton
                icon="trash-2"
                label={tr("Sign out")}
                testID={`account-remove-${p().id}-${a().id}`}
                onPress={async () => {
                  if (await ask({ title: tr("Sign %s out?", a().display), description: tr("Its tokens are deleted from this computer."), confirm: tr("Sign out"), danger: true })) {
                    await act("key", () => Subscriptions.signOut(kind(), a().id));
                    await loadProviders();
                  }
                }}
              />
            </If>
          </Row>
        )}
      </Each>
      <Row first={p().accounts.length === 0} class="min-h-[38px] py-[6px] pl-[10px] pr-[6px]">
        <TextButton label={tr("Add another account")} icon="add" testID={`account-add-${p().id}`} onPress={() => signingIn.set(kind())} />
      </Row>
    </List>
  );
}

/** Signs in to an account of a subscription: its page in the browser, or a code typed on it, while pansy waits. */
function SignInModal(props: { kind: SubscriptionKind }) {
  const sub = () => subscriptionOf(props.kind);
  /** ChatGPT signs in in the browser or by a code; GitHub by a code alone. */
  const byCode = signal(props.kind === "copilot");
  const flow = signal<SignInState | null>(null);
  const waiting = () => flow()?.state === "waiting";
  const close = () => {
    const f = flow();
    if (f && (f.state === "waiting" || f.state === "starting")) void Subscriptions.cancel(f.id);
    signingIn.set(null);
  };
  const finish = async (f: SignInState) => {
    toast(tr("Signed in to %s", f.user));
    signingIn.set(null);
    adding.set(false);
    await loadProviders();
    // Its plan's models.
    await Providers.refreshModels(f.kind).catch(() => 0);
    await loadProviders();
  };
  effect(() => {
    const f = flow();
    if (!f || f.state !== "waiting") return;
    const timer = setInterval(() => {
      const now = JSON.parse(Subscriptions.status(f.id)) as SignInState | null;
      if (!now || now.state === "waiting") return;
      flow.set(now);
      if (now.state === "done") void finish(now);
    }, 1000);
    return () => clearInterval(timer);
  });
  const start = () =>
    void act("sign-in", async () => {
      const f = JSON.parse(await Subscriptions.signIn(props.kind, byCode())) as SignInState;
      flow.set(f);
      if (f.code) await Clipboard.writeText(f.code).catch(() => {});
      void openUrl(f.url);
    });
  return (
    <Modal
      testID="sign-in-modal"
      title={tr("Sign in to %s", sub().name)}
      icon={<Logo name={logoNamed(sub().icon)} size={18} />}
      aside={<Text class="text-[11.5px] text-muted-foreground line-clamp-1">{sub().note}</Text>}
      onClose={close}
      foot={
        <>
          <View class="flex-1" />
          <TextButton label={tr("Cancel")} testID="sign-in-cancel" onPress={close} />
          <If when={!waiting()}>
            <PrimaryButton
              label={flow()?.state === "failed" ? tr("Try again") : tr("Sign in")}
              testID="sign-in-start"
              disabled={isBusy("sign-in")}
              onPress={start}
            />
          </If>
        </>
      }
    >
      <If when={props.kind === "codex" && !waiting()}>
        <FormRow label={tr("How")} hint={byCode() ? tr("For a browser that can't come back to this computer: type a code on ChatGPT's page") : tr("ChatGPT's page opens in your browser and comes back to pansy")}>
          <Segs
            testID="sign-in-how"
            value={byCode() ? "code" : "browser"}
            onChange={(v) => byCode.set(v === "code")}
            options={[
              { value: "browser", label: tr("In the browser") },
              { value: "code", label: tr("With a code") },
            ]}
          />
        </FormRow>
      </If>
      <If when={!flow() || flow()!.state === "canceled"}>
        <Text class="text-[12px] text-muted-foreground">
          {tr("pansy keeps the account in its logins.json, readable by you alone, and serves its plan's models as %s/…. An account %s is signed in to already is used as it is.", props.kind, AGENT_OF[props.kind])}
        </Text>
      </If>
      <If when={waiting()}>
        <If when={flow()!.code}>
          <FormRow label={tr("Code")} hint={tr("Copied: paste it on the page")}>
            <View class="flex-row items-center gap-3">
              <Text testID="sign-in-code" class="font-mono text-[20px] font-semibold tracking-[2px] text-foreground">{flow()!.code}</Text>
              <CopyButton label={tr("Copy the code")} onCopy={() => Clipboard.writeText(flow()!.code)} />
            </View>
          </FormRow>
        </If>
        <FormRow label={tr("Page")}>
          <View class="flex-row items-center gap-2">
            <Mono text={flow()!.url} class="flex-1 line-clamp-1" />
            <TextButton label={tr("Open ↗")} tone="link" testID="sign-in-open" onPress={() => void openUrl(flow()!.url)} />
            <CopyButton label={tr("Copy the page's address")} onCopy={() => Clipboard.writeText(flow()!.url)} />
          </View>
        </FormRow>
        <Text testID="sign-in-waiting" class="text-[12px] text-muted-foreground">{tr("Waiting for the sign-in to finish in the browser…")}</Text>
      </If>
      <If when={flow()?.state === "failed"}>
        <Text testID="sign-in-error" class="text-[12px] text-destructive">{flow()!.error}</Text>
      </If>
    </Modal>
  );
}

function Models(props: { provider: Provider }) {
  const p = () => props.provider;
  const newModel = signal("");
  const add = () => {
    const id = newModel().trim();
    if (!id) return;
    void act("model", () => Providers.addModel(p().id, id)).then((ok) => {
      if (ok) newModel.set("");
      void loadProviders();
    });
  };
  return (
    <View class="gap-2">
      <View class="flex-row flex-wrap gap-[6px]">
        <Each of={p().models} key={(m) => m.id}>
          {(m) => (
            <Hint text={m().hidden ? tr("Hidden from the pickers: click to show it") : tr("Click to hide it from the pickers")}>
              <View
                role="button"
                testID={`model-chip-${m().id}`}
                onPress={() => void act("model", () => Providers.setModelHidden(p().id, m().id, !m().hidden)).then(() => loadProviders())}
                class={`h-[24px] flex-row items-center gap-[6px] rounded-full border px-[10px] ${m().hidden ? "border-dashed border-border" : "border-border bg-card-2 hover:bg-pill"}`}
              >
                <Text class={`text-[12px] ${m().hidden ? "text-faint" : "text-foreground"}`}>{m().name || m().id}</Text>
                <If when={m().context}>
                  <Badge label={`${Math.round((m().context ?? 0) / 1024)}K`} />
                </If>
                <If when={m().manual}>
                  <View role="button" accessibilityLabel={tr("Remove model")} onPress={() => void act("model", () => Providers.removeModel(p().id, m().id)).then(() => loadProviders())}>
                    <Icon name="close" color="faint" size={9} />
                  </View>
                </If>
              </View>
            </Hint>
          )}
        </Each>
      </View>
      <View class="flex-row items-center gap-2">
        <Input value={newModel()} onChange={(v) => newModel.set(v)} onSubmit={add} mono testID={`model-new-${p().id}`} placeholder={tr("add a model id…")} class="w-[200px]" />
        <TextButton label={tr("Add model")} disabled={!newModel().trim()} onPress={add} />
        <TextButton
          label={isBusy(`models:${p().id}`) ? tr("Fetching…") : tr("Fetch models")}
          icon="sync"
          testID={`models-refresh-${p().id}`}
          disabled={isBusy(`models:${p().id}`)}
          onPress={() =>
            void act(`models:${p().id}`, async () => {
              const n = await Providers.refreshModels(p().id);
              toast(trn(n, "%s has %d model", "%s has %d models", p().name, n));
            }).then(() => loadProviders())
          }
        />
      </View>
    </View>
  );
}

/** A text field saved when Enter is pressed or Save is: the provider's name, address, proxy. */
function SavedInput(props: { value: string; onSave: (v: string) => Promise<unknown>; placeholder?: string; mono?: boolean; testID: string }) {
  const draft = signal(props.value);
  const changed = () => draft().trim() !== props.value.trim();
  const save = () => {
    if (changed()) void props.onSave(draft().trim());
  };
  return (
    <View class="flex-row items-center gap-2">
      <Input value={draft()} onChange={(v) => draft.set(v)} onSubmit={save} mono={props.mono} testID={props.testID} placeholder={props.placeholder} class="flex-1" />
      <If when={changed()}>
        <TextButton label={tr("Save")} tone="accent" testID={`${props.testID}-save`} onPress={save} />
      </If>
    </View>
  );
}

/** The headers sent with every request to the provider, with the ones its vendor documents to add. */
function Headers(props: { provider: Provider; hints: string[] }) {
  const p = () => props.provider;
  const rows = signal<[string, string][]>(Object.entries(p().headers));
  const saved = () => JSON.stringify(Object.entries(p().headers));
  const changed = () => JSON.stringify(rows().filter(([k]) => k.trim())) !== saved();
  const setRow = (i: number, row: [string, string]) => rows.update((all) => all.map((r, k) => (k === i ? row : r)));
  const save = () =>
    void act("headers", () => Providers.setHeaders(p().id, JSON.stringify(Object.fromEntries(rows().filter(([k]) => k.trim())))), tr("Headers saved")).then((ok) => {
      if (ok) void loadProviders();
    });
  const unused = () => props.hints.filter((h) => !rows().some(([k]) => k.toLowerCase() === h.toLowerCase()));
  return (
    <View class="gap-[6px]">
      <Each of={rows().map((r, i) => ({ r, i }))} key={(x) => x.i}>
        {(x) => (
          <View class="flex-row items-center gap-2">
            <Input value={x().r[0]} onChange={(v) => setRow(x().i, [v, x().r[1]])} mono testID={`header-name-${x().i}`} placeholder={tr("Header-Name")} class="w-[170px]" />
            <Input value={x().r[1]} onChange={(v) => setRow(x().i, [x().r[0], v])} onSubmit={save} mono testID={`header-value-${x().i}`} placeholder={tr("value")} class="flex-1" />
            <IconButton icon="close" label={tr("Remove header")} onPress={() => rows.update((all) => all.filter((_, k) => k !== x().i))} />
          </View>
        )}
      </Each>
      <View class="flex-row flex-wrap items-center gap-1">
        <TextButton label={tr("Add header")} icon="add" testID={`header-add-${p().id}`} onPress={() => rows.update((all) => [...all, ["", ""]])} />
        <Each of={unused()} key={(h) => h}>
          {(h) => <TextButton label={h()} icon="add" tone="muted" onPress={() => rows.update((all) => [...all, [h(), ""]])} />}
        </Each>
        <View class="flex-1" />
        <If when={changed()}>
          <TextButton label={tr("Save")} tone="accent" testID={`headers-save-${p().id}`} onPress={save} />
        </If>
      </View>
    </View>
  );
}

/** The header a key goes in on the Anthropic API, for a vendor that takes one alone. */
function KeyHeader(props: { provider: Provider }) {
  const p = () => props.provider;
  const value = () => p().anthropicAuth ?? "both";
  return (
    <FormRow
      label={tr("Key header")}
      hint={tr("Which header the API key goes in on the Anthropic API. Most services take both; pick one for a service that refuses the two together. pansy also switches to one by itself when a service says so.")}
    >
      <Segs
        testID={`key-header-${p().id}`}
        value={value()}
        onChange={(v) => void act("provider", () => Providers.setAuthHeader(p().id, v === "both" ? "" : v)).then(() => loadProviders())}
        options={[
          { value: "both", label: tr("Both") },
          { value: "x-api-key", label: "x-api-key" },
          { value: "bearer", label: "Authorization: Bearer" },
        ]}
      />
    </FormRow>
  );
}

/** A provider's editor. Every change is saved as it is made. */
function Editor(props: { provider: Provider }) {
  const p = () => props.provider;
  const preset = () => presetOf(p().preset);
  const close = () => editing.set(null);
  const site = () => p().website || p().keyUrl;
  /** Its key goes in the one header its vendor takes (Anthropic's own, Bedrock, Azure). */
  const vendorsHeader = () => p().anthropicAuth === "x-api-key" && p().authHeader !== "x-api-key";
  const anthropic = () => p().endpoints.some((e) => e.protocol === "anthropic");
  const sub = () => p().subscription;
  const custom = () => !p().preset && !sub();
  const placed = () => !!preset()?.endpoint || preset()?.kind === "local";
  const advanced = signal(false);
  const pickRegion = (id: string) =>
    void act("provider", () => Providers.setRegion(p().id, id)).then(async (ok) => {
      if (!ok) return;
      await loadProviders();
      // A plan or region that lists its models has them fetched.
      if (preset()?.regions.find((r) => r.id === id)?.lists) await Providers.refreshModels(p().id).catch(() => 0);
      await loadProviders();
    });
  return (
    <Modal
      testID="provider-editor"
      title={p().name}
      icon={<Logo name={logoNamed(p().icon)} size={18} />}
      aside={
        <View class="flex-row items-center gap-2">
          <If when={p().note}>
            <Text class="text-[11.5px] text-muted-foreground line-clamp-1">{trCore(p().note)}</Text>
          </If>
          <If when={site()}>
            <TextButton label={`${hostOf(site())} ↗`} tone="link" onPress={() => void openUrl(site())} />
          </If>
          <Switch
            checked={p().enabled}
            label={tr("Use %s", p().name)}
            onChange={(on) => void act("provider", () => Providers.setEnabled(p().id, on)).then(() => loadProviders())}
          />
        </View>
      }
      onClose={close}
      foot={
        <>
          <If when={!sub()}>
          <TextButton
            label={tr("Remove")}
            tone="danger"
            testID={`provider-remove-${p().id}`}
            onPress={async () => {
              const yes = await ask({
                title: tr("Remove %s?", p().name),
                description: tr("Its keys are deleted from this computer. Agents and groups that use its models stop working until you pick others."),
                confirm: tr("Remove"),
                danger: true,
              });
              if (yes) {
                close();
                await act("provider", () => Providers.remove(p().id), tr("%s removed", p().name));
                await loadProviders();
              }
            }}
          />
          </If>
          <View class="flex-1" />
          <PrimaryButton label={tr("Done")} testID="provider-done" onPress={close} />
        </>
      }
    >
      <If when={!p().enabled}>
        <Text testID="provider-off-note" class="text-[12px] text-muted-foreground">
          {tr("Switched off: agents aren't given its models and no request goes to it. Its keys and settings are kept; switch it on to use it again.")}
        </Text>
      </If>
      <If when={preset() && preset()!.regions.length > 0}>
        <FormRow label={trCore(preset()!.regionLabel)} hint={tr("which endpoint %s is reached through", preset()!.name)}>
          <RegionPicker testID={`provider-region-${p().id}`} regions={preset()!.regions} value={p().region} onChange={pickRegion} />
        </FormRow>
      </If>
      <FormRow label={tr("Agents")}>
        <If when={usersOf(p()).length > 0} else={<Text class="text-[12px] text-muted-foreground">{tr("No agent is on its models yet.")}</Text>}>
          <View class="flex-row flex-wrap gap-[6px]">
            <Each of={usersOf(p())} key={(a) => a.id}>
              {(a) => (
                <View class="h-[24px] flex-row items-center gap-[6px] rounded-[7px] bg-accent px-2">
                  <Logo name={agentLogo(a().id)} size={13} />
                  <Text class="text-[12px] text-foreground">{a().name}</Text>
                  <Mono text={choiceName(a().model)} class="text-[11px] text-primary" />
                </View>
              )}
            </Each>
          </View>
        </If>
      </FormRow>
      <FormRow label={tr("Name")}>
        <SavedInput
          value={p().name}
          placeholder={preset()?.name}
          testID={`provider-name-${p().id}`}
          onSave={(name) => act("provider", () => Providers.rename(p().id, name || preset()?.name || p().id)).then(() => loadProviders())}
        />
      </FormRow>
      <FormRow label={tr("ID")} hint={tr("Agents pick its models as %s/…", p().id)}>
        <Mono text={p().id} />
      </FormRow>
      <If when={placed()}>
        <FormRow
          label={preset()?.endpoint ? tr("Endpoint") : tr("Server address")}
          hint={preset()?.endpoint ? undefined : tr("Where the server listens; change the port, or give another computer's address")}
        >
          <SavedInput
            value={preset()?.endpoint ? (p().endpoints[0]?.baseUrl ?? "") : p().address}
            mono
            testID={`provider-place-${p().id}`}
            onSave={(address) => act("provider", () => Providers.setAddress(p().id, address)).then(() => loadProviders())}
          />
        </FormRow>
      </If>
      <If when={sub()}>
        <FormRow label={tr("Accounts")} top hint={tr("Tried in order; the next one takes over when one runs out.")}>
          <SignIns provider={p()} />
        </FormRow>
      </If>
      <If when={!sub()}>
        <FormRow label={tr("Keys")} top hint={p().keyless && !p().accounts.length ? undefined : tr("Tried in order; the next one takes over when one runs out.")}>
          <Keys provider={p()} />
          <If when={p().keyUrl}>
            <View class="flex-row">
              <TextButton label={tr("Get a key ↗")} tone="link" onPress={() => void openUrl(p().keyUrl)} />
            </View>
          </If>
        </FormRow>
      </If>
      <FormRow label={tr("Models")} top>
        <Models provider={p()} />
      </FormRow>
      <FormRow label={tr("Endpoints")} top>
        <If when={custom()} else={<EndpointList endpoints={p().endpoints} />}>
          <Each of={["chat", "responses", "anthropic", "gemini"] as Protocol[]} key={(x) => x}>
            {(protocol) => (
              <View class="flex-row items-center gap-3">
                <Text class="w-[64px] text-[11.5px] text-muted-foreground">{PROTOCOL_NAMES[protocol()]}</Text>
                <View class="flex-1">
                  <SavedInput
                    value={p().endpoints.find((e) => e.protocol === protocol())?.baseUrl ?? ""}
                    mono
                    placeholder={tr("none")}
                    testID={`endpoint-${protocol()}-${p().id}`}
                    onSave={(url) => act("provider", () => Providers.setEndpoint(p().id, protocol(), url)).then(() => loadProviders())}
                  />
                </View>
              </View>
            )}
          </Each>
        </If>
      </FormRow>
      <View class="flex-row">
        <TextButton label={tr("Advanced")} icon={advanced() ? "chev-down" : "chev-right"} testID={`provider-advanced-${p().id}`} onPress={() => advanced.update((a) => !a)} />
      </View>
      <If when={advanced()}>
        <FormRow
          label={tr("Headers")}
          top
          hint={tr("Optional headers sent with every request to %s, applied after its key. A header the agent's request has is replaced.", p().name)}
        >
          <Headers provider={p()} hints={preset()?.headerHints ?? []} />
        </FormRow>
        <If when={anthropic() && !vendorsHeader() && !sub()}>
          <KeyHeader provider={p()} />
        </If>
        <FormRow label={tr("Proxy")} hint={tr("An HTTP or SOCKS proxy for its requests; empty uses pansy's, in Settings.")}>
          <SavedInput
            value={p().proxy}
            mono
            placeholder="socks5://127.0.0.1:1080"
            testID={`provider-proxy-${p().id}`}
            onSave={(proxy) => act("provider", () => Providers.setProxy(p().id, proxy)).then(() => loadProviders())}
          />
        </FormRow>
      </If>
    </Modal>
  );
}

/** The logo of an app pansy imports from. */
const APP_LOGOS: Record<string, string> = { "claude-code": "claudecode-color", codex: "codex-color", opencode: "opencode", magpie: "magpie", "cc-switch": "cc-switch", alma: "alma" };

/** How one entry of the import comes in, as the user picked. */
type ImportPick = { on: boolean; mode: "add" | "replace" | "key"; name: string };

/**
 * Bringing over the providers other apps have set up, as magpie does: a tab
 * per app with how many it has to bring over, its entries with their keys
 * masked, and for one at an id in use or at an address pansy has, how it
 * comes in. pansy only reads the apps; the keys are read again when the
 * picks are imported, never handed to the window.
 */
function ImportModal(props: { only?: string[] }) {
  const sources = signal<ImportSource[] | null>(null);
  const failed = signal("");
  const picks = signal<Record<string, ImportPick>>({});
  const tab = signal("");
  const keyOf = (s: ImportSource, it: ImportItem) => `${s.id}\n${it.ref}`;
  void Providers.importSources()
    .then((json) => {
      const list = JSON.parse(json) as ImportSource[];
      const next: Record<string, ImportPick> = {};
      for (const s of list) {
        for (const it of s.items) {
          if (it.skip || it.status === "same") continue;
          const offered = !props.only || props.only.includes(it.fingerprint);
          next[keyOf(s, it)] = { on: offered && it.picked, mode: it.keyOf ? "key" : "add", name: "" };
        }
      }
      picks.set(next);
      const pickable = (s: ImportSource) => s.items.some((it) => next[keyOf(s, it)]?.on);
      tab.set((list.find(pickable) ?? list.find((s) => s.found && s.items.length) ?? list[0])?.id ?? "");
      sources.set(list);
    })
    .catch((e) => failed.set(String(e)));
  const close = () => importing.set(null);
  const set = (key: string, patch: Partial<ImportPick>) => picks.update((all) => ({ ...all, [key]: { ...all[key], ...patch } }));
  const count = () => Object.values(picks()).filter((x) => x.on).length;
  const open = () => sources()?.find((s) => s.id === tab());
  const go = () => {
    const chosen = Object.entries(picks())
      .filter(([, x]) => x.on)
      .map(([k, x]) => {
        const [source, ref] = k.split("\n");
        return { source, ref, mode: x.mode, name: x.name.trim() };
      });
    void act("import", async () => {
      const out = JSON.parse(await Providers.import(JSON.stringify(chosen))) as { names: string[]; ids: string[] };
      close();
      adding.set(false);
      toast(out.names.length ? tr("Imported %d: %s", out.names.length, out.names.join(", ")) : tr("Nothing new to import"));
      await Promise.all([loadProviders(), loadImportFound()]);
      // the vendors' own model lists, as for a provider added by hand
      for (const id of out.ids) void Providers.refreshModels(id).then(() => loadProviders()).catch(() => {});
    });
  };
  return (
    <Modal
      testID="import-modal"
      title={tr("Import from other apps")}
      onClose={close}
      foot={
        <>
          <Text class="text-[12px] text-muted-foreground">{count() ? tr("%d selected", count()) : ""}</Text>
          <View class="flex-1" />
          <TextButton label={tr("Cancel")} onPress={close} />
          <PrimaryButton label={tr("Import")} testID="import-go" disabled={!count() || isBusy("import")} onPress={go} />
        </>
      }
    >
      <If when={sources()} else={<Text class="text-[12.5px] text-muted-foreground">{failed() || tr("Reading other apps…")}</Text>}>
        <Text class="text-[12px] text-muted-foreground">{tr("pansy reads these apps' settings and changes nothing in them. Pick the providers to bring over.")}</Text>
        <View class="flex-row flex-wrap gap-[6px]">
          <Each of={sources()!} key={(s) => s.id}>
            {(s) => {
              const n = () => s().items.filter((it) => picks()[keyOf(s(), it)]).length;
              const there = () => s().found && !s().error;
              return (
                <Hint text={!s().found ? tr("Not found on this computer") : s().error || trn(n(), "%d provider to bring over", "%d providers to bring over", n())}>
                  <View
                    role="tab"
                    testID={`import-tab-${s().id}`}
                    onPress={() => tab.set(s().id)}
                    class={`h-[28px] flex-row items-center gap-[6px] rounded-[7px] border px-[8px] ${tab() === s().id ? "border-primary bg-pill" : "border-border hover:bg-pill"}`}
                    style={{ opacity: there() ? 1 : 0.55 }}
                  >
                    <Logo name={logoNamed(APP_LOGOS[s().id])} size={14} />
                    <Text class="text-[12.5px] text-foreground">{s().name}</Text>
                    <Text class="text-[11.5px] text-faint">{there() ? String(n()) : "–"}</Text>
                  </View>
                </Hint>
              );
            }}
          </Each>
        </View>
        <If when={open()}>
          <ImportSourceView source={open()!} picks={picks()} keyOf={keyOf} set={set} />
        </If>
      </If>
    </Modal>
  );
}

/** One app of the import: where it was read, and its entries. */
function ImportSourceView(props: {
  source: ImportSource;
  picks: Record<string, ImportPick>;
  keyOf: (s: ImportSource, it: ImportItem) => string;
  set: (key: string, patch: Partial<ImportPick>) => void;
}) {
  const s = () => props.source;
  const mine = () => s().items.map((it) => props.keyOf(s(), it)).filter((k) => props.picks[k]);
  const on = () => mine().filter((k) => props.picks[k].on).length;
  return (
    <View testID={`import-source-${s().id}`} class="gap-2">
      <View class="flex-row items-center gap-2">
        <Logo name={logoNamed(APP_LOGOS[s().id])} size={16} />
        <Text class="text-[13px] font-semibold text-foreground">{s().name}</Text>
        <Code text={s().path} class="min-w-0 shrink" />
        <View class="flex-1" />
        <If when={mine().length > 0}>
          <View class="flex-row items-center gap-[6px]">
            <Check
              checked={on() > 0 && on() === mine().length}
              indeterminate={on() > 0 && on() < mine().length}
              label={tr("Select all")}
              testID={`import-all-${s().id}`}
              onChange={(all) => mine().forEach((k) => props.set(k, { on: all }))}
            />
            <Text class="text-[12px] text-muted-foreground">{tr("Select all")}</Text>
          </View>
        </If>
      </View>
      <If when={!s().found}>
        <Text class="text-[12.5px] text-muted-foreground">{tr("Not found on this computer")}</Text>
      </If>
      <If when={s().found && s().error}>
        <Text class="text-[12.5px] text-destructive">{s().error ?? ""}</Text>
      </If>
      <If when={s().found && !s().error && s().items.length === 0}>
        <Text class="text-[12.5px] text-muted-foreground">{tr("No providers in it")}</Text>
      </If>
      <If when={s().items.length > 0}>
        <List>
          <Each of={s().items} key={(it) => it.ref}>
            {(it, i) => <ImportRow source={s()} item={it()} first={i() === 0} pick={props.picks[props.keyOf(s(), it())]} set={(patch) => props.set(props.keyOf(s(), it()), patch)} />}
          </Each>
        </List>
      </If>
    </View>
  );
}

/** One entry of an app: its provider, how it stands against pansy's, and how it comes in. */
function ImportRow(props: { source: ImportSource; item: ImportItem; first: boolean; pick?: ImportPick; set: (patch: Partial<ImportPick>) => void }) {
  const it = () => props.item;
  const renamed = () => !!props.pick?.name.trim() && props.pick.name.trim() !== it().name;
  const sub = () => {
    if (it().skip) return trCore(it().skip);
    const bits = [it().host, it().key];
    if (it().keys > 1) bits.push(tr("%d keys", it().keys));
    if (it().models) bits.push(trn(it().models, "%d model", "%d models", it().models));
    return bits.filter(Boolean).join(" · ");
  };
  const choices = () => {
    const out: { value: ImportPick["mode"]; label: string }[] = [];
    if (it().keyOf) out.push({ value: "key", label: tr("Add as another key") });
    out.push({ value: "add", label: it().status === "taken" && !it().reserved && !renamed() ? tr("Keep both") : tr("Add as a new provider") });
    if (it().status === "taken" && !it().reserved && !renamed()) out.push({ value: "replace", label: tr("Replace it") });
    return out;
  };
  const tag = () =>
    it().status === "same" ? tr("Already added") : it().skip ? tr("Can't import") : it().status === "taken" ? tr("Name in use") : it().keyOf ? tr("Same vendor") : "";
  return (
    <Row first={props.first} testID={`import-${props.source.id}-${it().ref}`} class="items-start py-[10px]">
      <View class="pt-[2px]">
        <Check checked={!!props.pick?.on} disabled={!props.pick} label={it().name} testID={`import-pick-${props.source.id}-${it().ref}`} onChange={(on) => props.set({ on })} />
      </View>
      <View class="pt-[1px]">
        <Logo name={logoNamed(it().icon)} size={18} />
      </View>
      <View class="min-w-0 flex-1 gap-[3px]" style={{ opacity: props.pick ? 1 : 0.6 }}>
        <View class="flex-row items-center gap-2">
          <Text class="min-w-0 shrink text-[13px] font-medium text-foreground line-clamp-1">{it().name || it().ref}</Text>
          <If when={it().from}>
            <Text class="text-[11.5px] text-faint line-clamp-1">{it().from}</Text>
          </If>
        </View>
        <Text class="text-[12px] text-muted-foreground line-clamp-1">{sub()}</Text>
        <If when={props.pick && it().off}>
          <Text class="text-[12px] text-muted-foreground">{trCore(it().off)}</Text>
        </If>
        <If when={it().status === "same" && it().existing}>
          <Text class="text-[12px] text-muted-foreground">{tr("pansy has %s already", it().existing)}</Text>
        </If>
        <If when={props.pick && (it().keyOf || it().status === "taken")}>
          <Text class="text-[12px] text-muted-foreground">
            {it().reserved ? tr("%s is a subscription in pansy; this one comes in beside it as a provider of its own", it().existing) : tr("pansy has %s already", it().existing)}
          </Text>
          <View class="flex-row flex-wrap items-center gap-2 pt-[2px]">
            <If when={it().status === "taken"}>
              <Input
                value={props.pick?.name || it().name}
                onChange={(v) => {
                  const next = v.trim() && v.trim() !== it().name;
                  props.set({ name: v, ...(next ? { on: true, ...(props.pick?.mode === "replace" ? { mode: "add" as const } : {}) } : {}) });
                }}
                testID={`import-name-${props.source.id}-${it().ref}`}
                placeholder={tr("My relay")}
                class="h-[26px] w-[180px]"
              />
            </If>
            <If when={choices().length > 1}>
              <Segs small testID={`import-mode-${props.source.id}-${it().ref}`} value={props.pick!.mode} onChange={(mode) => props.set({ mode, on: true })} options={choices()} />
            </If>
          </View>
        </If>
      </View>
      <If when={tag()}>
        <Badge label={tag()} />
      </If>
    </Row>
  );
}

/** What the Providers page found set up in other apps by itself, with a way to import it or ignore it. */
function FoundOffer(props: { compact: boolean }) {
  const n = () => importFound().length;
  const sources = () => [...new Set(importFound().map((c) => c.source))].join(" · ");
  const review = () => importing.set({ only: importFound().map((c) => c.fingerprint) });
  const ignore = () =>
    void act("import-ignore", () => Providers.importIgnore(JSON.stringify(importFound().map((c) => c.fingerprint)))).then(() => loadImportFound());
  return (
    <If
      when={props.compact}
      else={
        <View testID="import-found" class="flex-row items-center gap-3 rounded-[12px] border border-border bg-card px-4 py-3">
          <Icon name="sparkles" color="primary" size={16} />
          <View class="min-w-0 flex-1 gap-[2px]">
            <Text class="text-[13px] font-medium text-foreground">{trn(n(), "Found %d local provider configuration", "Found %d local provider configurations", n())}</Text>
            <Text class="text-[12px] text-muted-foreground line-clamp-1">{sources()}</Text>
          </View>
          <Hint text={tr("Hide these configurations until they change")}>
            <TextButton label={tr("Ignore")} testID="import-found-ignore" onPress={ignore} />
          </Hint>
          <PrimaryButton label={tr("Review and import…")} testID="import-found-review" onPress={review} />
        </View>
      }
    >
      <Hint text={sources()}>
        <TextButton label={tr("Import local configurations (%d)…", n())} icon="sparkles" tone="accent" testID="import-found-review" onPress={review} />
      </Hint>
      <Hint text={tr("Hide these configurations until they change")}>
        <TextButton label={tr("Ignore")} testID="import-found-ignore" onPress={ignore} />
      </Hint>
    </If>
  );
}

/**
 * What an import link would add, for the user to check, as magpie shows it:
 * where the prompts and the key go, its name and key to change, its
 * endpoints and models, and the provider it would replace. Nothing is saved
 * until Add; the link is read again then, its key never shown whole.
 */
function LinkModal(props: { link: string }) {
  const view = signal<LinkView | null>(null);
  const name = signal("");
  const key = signal("");
  void readPaste(props.link)
    .then((c) => {
      const v = c.link ?? { error: tr("This is not an import link."), id: "", name: "", icon: "generic", endpoints: [], hosts: [], key: "", keyless: false, keyUrl: "", models: [], replaces: "" };
      name.set(v.name);
      view.set(v);
    })
    .catch((e) => view.set({ error: String(e), id: "", name: "", icon: "generic", endpoints: [], hosts: [], key: "", keyless: false, keyUrl: "", models: [], replaces: "" }));
  const close = () => linkOpen.set(null);
  const v = () => view();
  const ready = () => !!v() && !v()!.error && !!name().trim() && (!!v()!.key || v()!.keyless || !!key().trim());
  const add = () => {
    void act("link", async () => {
      const out = JSON.parse(await Providers.addLink(props.link, name().trim(), key().trim())) as { names: string[]; ids: string[] };
      close();
      adding.set(false);
      toast(tr("%s added", out.names.join(", ")));
      await loadProviders();
      for (const id of out.ids) void Providers.refreshModels(id).then(() => loadProviders()).catch(() => {});
    });
  };
  return (
    <Modal
      testID="link-modal"
      title={v() && !v()!.error ? v()!.name : tr("Import link")}
      icon={<Logo name={logoNamed(v()?.icon)} size={18} />}
      onClose={close}
      foot={
        <>
          <View class="flex-1" />
          <TextButton label={v()?.error ? tr("Close") : tr("Cancel")} onPress={close} />
          <If when={v() && !v()!.error}>
            <PrimaryButton label={v()?.replaces ? tr("Replace") : tr("Add")} testID="link-add" disabled={!ready() || isBusy("link")} onPress={add} />
          </If>
        </>
      }
    >
      <If when={v()} else={<Text class="text-[12.5px] text-muted-foreground">{tr("Reading the link…")}</Text>}>
        <If
          when={!v()!.error}
          else={
            <Text testID="link-error" class="rounded-[8px] bg-warning-soft px-3 py-2 text-[12px] leading-[18px] text-warning">
              {tr("This link can't be imported: %s", trCore(v()!.error ?? ""))}
            </Text>
          }
        >
          <Text testID="link-warning" class="rounded-[8px] bg-warning-soft px-3 py-2 text-[12px] leading-[18px] text-warning">
            {tr("Added from a link. Your prompts and this key will go to %s; add it only if you trust the site that sent you here.", v()!.hosts.join(", "))}
          </Text>
          <FormRow label={tr("Name")}>
            <Input value={name()} onChange={(x) => name.set(x)} testID="link-name" placeholder={tr("My relay")} />
          </FormRow>
          <FormRow label={tr("API key")} hint={v()!.key ? tr("From the link. Kept in pansy's providers.json, readable by you alone.") : ""}>
            <Input
              value={key()}
              onChange={(x) => key.set(x)}
              masked
              mono
              testID="link-key"
              placeholder={v()!.key ? tr("%s · from the link, or paste another", v()!.key) : v()!.keyless ? tr("optional for local servers") : tr("paste an API key")}
            />
            <If when={!v()!.key && v()!.keyUrl}>
              <View class="flex-row">
                <TextButton label={tr("Get a key ↗")} tone="link" onPress={() => void openUrl(v()!.keyUrl)} />
              </View>
            </If>
          </FormRow>
          <FormRow label={tr("Endpoints")} top>
            <Each of={v()!.endpoints} key={(e) => e.protocol}>
              {(e) => (
                <View class="flex-row items-center gap-3">
                  <Text class="w-[64px] text-[11.5px] text-muted-foreground">{PROTOCOL_NAMES[e().protocol]}</Text>
                  <Mono text={e().url} class="flex-1" />
                </View>
              )}
            </Each>
          </FormRow>
          <If when={v()!.models.length > 0}>
            <FormRow label={tr("Models")}>
              <Text class="text-[12px] text-fg-2">{v()!.models.join(", ")}</Text>
            </FormRow>
          </If>
          <If when={v()!.replaces}>
            <Text testID="link-replaces" class="text-[12px] text-warning">
              {tr("Replaces your %s, key and all.", v()!.replaces)}
            </Text>
          </If>
        </If>
      </If>
    </Modal>
  );
}

/**
 * Paste a configuration: a curl command from a vendor's docs, the
 * environment lines its page gives (export ANTHROPIC_BASE_URL=…), or an
 * import link, in a box of several lines; what it reads as is said under
 * it, and Continue opens the custom form filled in or the link's
 * confirmation. It starts with what the clipboard holds when it is one.
 */
function PasteModal() {
  const text = signal("");
  const read = signal<Pasted | null>(null);
  let asked = 0;
  const look = (v: string) => {
    text.set(v);
    const n = ++asked;
    if (!v.trim()) {
      read.set(null);
      return;
    }
    void readPaste(v)
      .then((c) => {
        if (n === asked) read.set(c);
      })
      .catch(() => {});
  };
  void Clipboard.readText()
    .then((t) => {
      if (!text() && looksPasted(t)) look(t);
    })
    .catch(() => {});
  const close = () => pasting.set(false);
  const c = () => read();
  const usable = () => !!c() && (c()!.kind === "link" || ((c()!.kind === "curl" || c()!.kind === "env") && c()!.bases.length > 0));
  const said = () => {
    const x = c();
    if (!x || !text().trim()) return "";
    if (x.kind === "link") return x.link?.error ? tr("This link can't be imported: %s", trCore(x.link.error)) : tr("An import link for %s.", x.link?.name ?? "");
    if (x.kind === "curl" || x.kind === "env") {
      if (!x.bases.length) return x.kind === "curl" ? tr("Its URL isn't an address ending in /chat/completions, /messages or /responses: type the base URL.") : tr("It names no base URL, only a key.");
      return pastedNote(x);
    }
    return tr("pansy reads a curl command, environment lines (ANTHROPIC_BASE_URL=…) or a pansy:// or magpie:// import link.");
  };
  const go = () => {
    const t = text();
    close();
    void openPasted(t);
  };
  return (
    <Modal
      testID="paste-modal"
      title={tr("Paste a configuration")}
      onClose={close}
      foot={
        <>
          <View class="flex-1" />
          <TextButton label={tr("Cancel")} onPress={close} />
          <PrimaryButton label={tr("Continue")} testID="paste-go" disabled={!usable()} onPress={go} />
        </>
      }
    >
      <Text class="text-[12px] text-muted-foreground">{tr("A curl command from a vendor's docs, the environment lines its page gives, or an import link. The key stays on this computer.")}</Text>
      <Input value={text()} onChange={look} multiline mono autoFocus testID="paste-text" placeholder={'export ANTHROPIC_BASE_URL="https://…"\nexport ANTHROPIC_AUTH_TOKEN="sk-…"'} />
      <If when={said()}>
        <Text testID="paste-said" class={`text-[12px] leading-[18px] ${usable() ? "text-fg-2" : "text-warning"}`}>
          {said()}
        </Text>
      </If>
    </Modal>
  );
}

/** The modals of this page, over the whole window (App.tsx). */
export function ProviderModals() {
  const open = () => providers().find((p) => p.id === editing());
  return (
    <>
      <If when={open()}>
        <Editor provider={open()!} />
      </If>
      <If when={chosen() === "custom"}>
        <AddCustom />
      </If>
      <If when={chosen() && chosen() !== "custom"}>
        <AddPreset preset={chosen() as Preset} />
      </If>
      <If when={signingIn()}>
        <SignInModal kind={signingIn()!} />
      </If>
      <If when={pasting()}>
        <PasteModal />
      </If>
      <If when={linkOpen()}>
        <LinkModal link={linkOpen()!} />
      </If>
      <If when={importing()}>
        <ImportModal only={importing()!.only} />
      </If>
    </>
  );
}

export function ProvidersPage() {
  const words = () => finding().trim().toLowerCase().split(/\s+/).filter(Boolean);
  const listed = (on: boolean) => providers().filter((p) => p.enabled === on && matches(p, words()));
  /** The ones switched off show while searching, whatever the fold says. */
  const offShown = () => !offFolded() || words().length > 0;
  const off = () => providers().filter((p) => !p.enabled);
  return (
    <Page testID="page-providers">
      <If when={providers().length > 0}>
        <RowHead>
          <Input value={finding()} onChange={(v) => finding.set(v)} testID="provider-find" placeholder={tr("Find a provider")} class="w-[220px]" />
        </RowHead>
        <If when={listed(true).length > 0}>
          <List testID="providers">
            <Each of={listed(true)} key={(p) => p.id}>
              {(p, i) => <ProviderRow provider={p()} first={i() === 0} />}
            </Each>
          </List>
        </If>
        <If when={off().length > 0}>
          <RowHead label={tr("Switched off")} note={String(words().length ? listed(false).length : off().length)}>
            <If when={!words().length}>
              <TextButton
                label={offFolded() ? tr("Show") : tr("Hide")}
                icon={offFolded() ? "chev-right" : "chev-down"}
                testID="providers-off-fold"
                onPress={() => offFolded.update((f) => !f)}
              />
            </If>
          </RowHead>
          <If when={offShown() && listed(false).length > 0}>
            <List testID="providers-off">
              <Each of={listed(false)} key={(p) => p.id}>
                {(p, i) => <ProviderRow provider={p()} first={i() === 0} />}
              </Each>
            </List>
          </If>
        </If>
        <If when={words().length > 0 && listed(true).length + listed(false).length === 0}>
          <Text testID="provider-none" class="px-[6px] text-[12.5px] text-muted-foreground">
            {tr("No provider matches “%s”", finding().trim())}
          </Text>
        </If>
      </If>
      <If when={providers().length === 0 && importFound().length > 0}>
        <FoundOffer compact={false} />
      </If>
      <If when={providers().length === 0 && !adding()}>
        <View class="items-center gap-2 rounded-[12px] border border-dashed border-border px-6 py-8">
          <Text class="text-[13px] font-medium text-foreground">{tr("No providers yet")}</Text>
          <Text class="text-center text-[12px] text-muted-foreground">{tr("Add DeepSeek, Kimi, GLM, OpenAI, Anthropic, Gemini, OpenRouter or any compatible URL.")}</Text>
        </View>
      </If>
      <If when={!adding()}>
        <View class="flex-row px-[2px]">
          <TextButton label={tr("Add provider")} icon="add" testID="provider-add" onPress={() => adding.set(true)} />
          <If when={providers().length > 0 && importFound().length > 0}>
            <View class="flex-1" />
            <FoundOffer compact />
          </If>
        </View>
      </If>
      <If when={adding()}>
        <AddSection />
      </If>
    </Page>
  );
}
