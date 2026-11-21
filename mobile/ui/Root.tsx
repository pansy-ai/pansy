// The app: two stacks of pages (对话, 设置) under a tab bar of icons, PR and
// 用量 dimmed until phase 2, and what opens over everything: adding a
// machine, a new task, renaming, the toasts. The tab bar is T3's on iOS 26:
// a glass capsule floating over the pages, the chosen tab on a platter, and
// compose in a glass circle beside it. It is drawn here on every platform
// (Liquid Glass where the system has it): the system's bar has no dimmed
// tabs, no titles-free look and no room for compose.
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
import { chats, newTask, openThread, pairLink, pairOpen, settingsNav, tab, type Tab } from "./nav";
import { machines } from "../state/store";
import { NewTask } from "./NewTask";
import { PairSheet } from "./Pair";
import { GLASS, Icon, LIQUID, MOTION, SHADOW, toast, Toasts, type IconName } from "./parts";
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
      class={`h-[52px] w-[62px] items-center justify-center rounded-full ${on() ? "bg-line" : ""} ${MOTION}`}
      style={{ opacity: props.disabled ? 0.35 : 1 }}
    >
      <Icon name={props.icon} color={on() ? "primary" : "fg"} size={24} />
    </View>
  );
}

function TabBar() {
  const hidden = () => tab() === "chats" && chats.routes().length > 1;
  return (
    <If when={!hidden()}>
      <View class="absolute bottom-0 left-0 right-0 flex-row items-center justify-between px-5 pb-[env(safe-area-inset-bottom)]">
        <View role="tablist" class={`mb-2 flex-row rounded-full p-1 ${GLASS}`} style={{ boxShadow: SHADOW.glass(), ...LIQUID }}>
          <TabButton icon="message-square" label={tr("Chats")} tab="chats" />
          <TabButton icon="git-pull-request" label={tr("Pull requests")} disabled />
          <TabButton icon="chart-column" label={tr("Usage")} disabled />
          <TabButton icon="settings" label={tr("Settings")} tab="settings" />
        </View>
        <View
          role="button"
          accessibilityLabel={tr("New task")}
          testID="new-task"
          onPress={() => (machines().length ? newTask.set({}) : pairOpen.set(true))}
          class={`mb-2 h-[62px] w-[62px] items-center justify-center rounded-full ${GLASS} active:scale-90 ${MOTION}`}
          style={{ boxShadow: SHADOW.glass(), ...LIQUID }}
        >
          <Icon name="square-pen" size={24} />
        </View>
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
