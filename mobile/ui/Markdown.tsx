// A small Markdown for the agents' answers, flat on the canvas: paragraphs,
// headings, lists, quotes, fenced code (its language and a copy button) and
// inline code, bold, italics and links. Tables come as code, as typed.
import { Clipboard, openUrl, Scroll, Text, View, type JSX } from "nsh";
import { Haptics } from "nsh/mobile";
import { tr } from "./i18n";
import { Icon, toast } from "./parts";

export type Block =
  | { kind: "p"; text: string }
  | { kind: "h"; level: number; text: string }
  | { kind: "list"; ordered: boolean; items: string[]; start: number }
  | { kind: "quote"; text: string }
  | { kind: "code"; lang: string; text: string }
  | { kind: "rule" };

/** `source`'s blocks; an unclosed fence (still streaming) runs to the end. */
export function blocks(source: string): Block[] {
  const lines = source.replace(/\r\n?/g, "\n").split("\n");
  const out: Block[] = [];
  let para: string[] = [];
  const flush = () => {
    if (para.length) out.push({ kind: "p", text: para.join("\n") });
    para = [];
  };
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const fence = /^\s*(```|~~~)\s*([\w+#.-]*)/.exec(line);
    if (fence) {
      flush();
      const code: string[] = [];
      i++;
      while (i < lines.length && !lines[i].trim().startsWith(fence[1])) code.push(lines[i++]);
      out.push({ kind: "code", lang: fence[2], text: code.join("\n") });
      continue;
    }
    if (/^\s*$/.test(line)) {
      flush();
      continue;
    }
    const h = /^(#{1,6})\s+(.*)$/.exec(line);
    if (h) {
      flush();
      out.push({ kind: "h", level: h[1].length, text: h[2] });
      continue;
    }
    if (/^\s*([-*_])(\s*\1){2,}\s*$/.test(line)) {
      flush();
      out.push({ kind: "rule" });
      continue;
    }
    const item = /^\s*([-*+]|(\d+)[.)])\s+(.*)$/.exec(line);
    if (item) {
      flush();
      const ordered = !!item[2];
      const last = out[out.length - 1];
      if (last?.kind === "list" && last.ordered === ordered) last.items.push(item[3]);
      else out.push({ kind: "list", ordered, items: [item[3]], start: ordered ? Number(item[2]) : 1 });
      continue;
    }
    const quote = /^\s*>\s?(.*)$/.exec(line);
    if (quote) {
      flush();
      const last = out[out.length - 1];
      if (last?.kind === "quote") last.text += `\n${quote[1]}`;
      else out.push({ kind: "quote", text: quote[1] });
      continue;
    }
    // a list item's continuation line
    const last = out[out.length - 1];
    if (!para.length && last?.kind === "list" && /^\s{2,}/.test(line)) {
      last.items[last.items.length - 1] += ` ${line.trim()}`;
      continue;
    }
    para.push(line);
  }
  flush();
  return out;
}

export type Span = { text: string; code?: boolean; bold?: boolean; em?: boolean; link?: string };

/** The inline spans of a block's text. */
export function spans(text: string): Span[] {
  const out: Span[] = [];
  const re = /(`+)([^`]+?)\1|\*\*([^*]+)\*\*|__([^_]+)__|\*([^*\s][^*]*)\*|\[([^\]]+)\]\(([^)\s]+)\)/g;
  let at = 0;
  for (let m = re.exec(text); m; m = re.exec(text)) {
    if (m.index > at) out.push({ text: text.slice(at, m.index) });
    if (m[2] !== undefined) out.push({ text: m[2], code: true });
    else if (m[3] !== undefined || m[4] !== undefined) out.push({ text: m[3] ?? m[4], bold: true });
    else if (m[5] !== undefined) out.push({ text: m[5], em: true });
    else out.push({ text: m[6], link: m[7] });
    at = m.index + m[0].length;
  }
  if (at < text.length) out.push({ text: text.slice(at) });
  return out;
}

function Inline(props: { text: string; class?: string }) {
  return (
    <Text class={`text-[16px] leading-[23px] text-fg ${props.class ?? ""}`} style={{ userSelect: "text" }}>
      <Each of={spans(props.text)} key={(s, i) => `${i}:${JSON.stringify(s)}`}>
        {(s) => (
          <If
            when={s().code}
            else={
              <Text
                class={`${s().bold ? "font-bold" : ""} ${s().em ? "italic" : ""} ${s().link ? "text-primary underline" : ""}`}
                onPress={s().link ? () => void openUrl(s().link!).catch(() => {}) : undefined}
              >
                {s().text}
              </Text>
            }
          >
            <Text class="rounded-[5px] bg-grouped font-mono text-[13.5px]">{` ${s().text} `}</Text>
          </If>
        )}
      </Each>
    </Text>
  );
}

function Code(props: { lang: string; text: string }) {
  const copy = async () => {
    try {
      await Clipboard.writeText(props.text);
      Haptics.selection();
      toast(tr("Copied"));
    } catch {
      toast(tr("Couldn't copy"), { tone: "error" });
    }
  };
  return (
    <View class="overflow-hidden rounded-lg bg-grouped">
      <View class="h-7 flex-row items-center px-[10px]">
        <Text class="flex-1 font-mono text-[11px] text-fg-3">{props.lang}</Text>
        <View role="button" accessibilityLabel={tr("Copy")} onPress={() => void copy()} hitSlop={8} class="active:opacity-50">
          <Icon name="copy" color="fg-3" size={13} />
        </View>
      </View>
      <Scroll horizontal scrollbar={false}>
        <Text class="px-[10px] pb-2 font-mono text-[12.5px] leading-[18px] text-fg" style={{ userSelect: "text" }}>
          {props.text}
        </Text>
      </Scroll>
    </View>
  );
}

export function Markdown(props: { text: string; testID?: string }) {
  const render = (b: Block): JSX.Element => {
    switch (b.kind) {
      case "p":
        return <Inline text={b.text} />;
      case "h":
        return <Inline text={b.text} class={b.level <= 1 ? "text-[21px] leading-[28px] font-bold" : b.level === 2 ? "text-[18px] font-bold" : "font-bold"} />;
      case "list":
        return (
          <View class="gap-1">
            <Each of={b.items} key={(item, i) => `${i}:${item}`}>
              {(item, i) => (
                <View class="flex-row gap-2 pl-1">
                  <Text class="w-[18px] text-right text-[16px] leading-[23px] text-fg-2">{b.ordered ? `${b.start + i()}.` : "•"}</Text>
                  <View class="min-w-0 flex-1">
                    <Inline text={item()} />
                  </View>
                </View>
              )}
            </Each>
          </View>
        );
      case "quote":
        return (
          <View class="border-l-[3px] border-border pl-3">
            <Inline text={b.text} class="text-fg-2" />
          </View>
        );
      case "code":
        return <Code lang={b.lang} text={b.text} />;
      default:
        return <View class="h-px bg-line" />;
    }
  };
  return (
    <View testID={props.testID} class="gap-2 px-1">
      <Each of={blocks(props.text)} key={(b, i) => `${i}:${JSON.stringify(b)}`}>
        {(b) => render(b())}
      </Each>
    </View>
  );
}
