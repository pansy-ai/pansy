// The app: two stacks of pages (对话, 设置) under a tab bar of icons, PR and
// 用量 dimmed until phase 2, and what opens over everything: adding a
// machine, a new task, renaming, the toasts. The tab bar is drawn here on
// every platform: the system's has no dimmed tabs, and the design's has no
// titles.
import { UrlScheme, View } from "nsh";
import { NavStack } from "nsh/base";
import { Haptics } from "nsh/mobile";
import "../assets/fonts/DMSans-400.woff2";
import "../assets/fonts/DMSans-500.woff2";
import "../assets/fonts/DMSans-700.woff2";
import { clearPageSecret, pageUrl, parsePairUrl } from "../net/pair";
import { loadPrefs } from "../state/prefs";
import { start } from "../state/store";
import { RenameSheet } from "./actions";
import { loadLanguage, tr } from "./i18n";
import { chats, openThread, pairLink, pairOpen, settingsNav, tab, type Tab } from "./nav";
import { NewTask } from "./NewTask";
import { PairSheet } from "./Pair";
import { BLUR, Icon, toast, Toasts, type IconName } from "./parts";
import { AboutPage, MachinePage, SettingsRoot } from "./Settings";
import { ThreadPage } from "./Thread";
import { ThreadsPage } from "./Threads";

/** A link that opens the app: a pairing link, or a thread's (`pansy://thread/<machine>/<thread>`). */
function follow(url: string) {
  const p = parsePairUrl(url);
  if (p) {
    pairLink.set(p);
    pairOpen.set(true);
    return;
  }
  const t = /^pansy:\/\/thread\/([^/]+)\/([^/?#]+)/.exec(url);
  if (t) openThread(decodeURIComponent(t[1]), decodeURIComponent(t[2]));
}

function TabButton(props: { icon: IconName; label: string; tab?: Tab; disabled?: boolean }) {
  const on = () => !!props.tab && tab() === props.tab;
  return (
    <View
      role="tab"
      accessibilityLabel={props.label}
      accessibilityState={{ selected: on(), disabled: props.disabled }}
      testID={`tab-${props.tab ?? props.icon}`}
      onPress={() => {
        if (props.disabled) {
          toast(tr("%s comes in phase 2", props.label));
          return;
        }
        if (props.tab === tab()) (props.tab === "chats" ? chats : settingsNav).popToRoot();
        else if (props.tab) tab.set(props.tab);
        Haptics.selection();
      }}
      class="h-[50px] w-16 items-center justify-center"
      style={{ opacity: props.disabled ? 0.35 : 1 }}
    >
      <Icon name={props.icon} color={on() ? "primary" : "fg-3"} size={26} />
    </View>
  );
}

function TabBar() {
  const hidden = () => tab() === "chats" && chats.routes().length > 1;
  return (
    <If when={!hidden()}>
      <View class="flex-row justify-around border-t border-line bg-glass px-10 pb-[env(safe-area-inset-bottom)] pt-1" style={BLUR}>
        <TabButton icon="message-square" label={tr("Chats")} tab="chats" />
        <TabButton icon="git-pull-request" label={tr("Pull requests")} disabled />
        <TabButton icon="chart-column" label={tr("Usage")} disabled />
        <TabButton icon="settings" label={tr("Settings")} tab="settings" />
      </View>
    </If>
  );
}

export function Root() {
  void loadLanguage();
  void loadPrefs();
  void start();
  // the web app opened from the computer's QR code: `#pair=` in the page's address
  const page = pageUrl();
  if (page && parsePairUrl(page)) {
    follow(page);
    clearPageSecret();
  } else {
    const url = UrlScheme.url?.();
    if (url) follow(url);
  }
  UrlScheme.on("open", ({ url }) => follow(url));
  return (
    <View class="flex-1 bg-screen font-sans">
      <View class="flex-1" style={{ display: tab() === "chats" ? "flex" : "none" }}>
        <NavStack state={chats} style={{ flex: 1 }}>
          {(r) => (r.name === "thread" ? <ThreadPage machine={r.machine} thread={r.thread} /> : <ThreadsPage />)}
        </NavStack>
      </View>
      <View class="flex-1" style={{ display: tab() === "settings" ? "flex" : "none" }}>
        <NavStack state={settingsNav} style={{ flex: 1 }}>
          {(r) => (r.name === "machine" ? <MachinePage id={r.id} /> : r.name === "about" ? <AboutPage /> : <SettingsRoot />)}
        </NavStack>
      </View>
      <TabBar />
      <Toasts />
      <PairSheet />
      <NewTask />
      <RenameSheet />
    </View>
  );
}
