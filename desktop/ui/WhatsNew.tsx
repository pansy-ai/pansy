// What's new, as magpie's: what a release says changed. Shown once after
// pansy restarted into an update, and from Settings › About › What's new,
// where an update not put in yet says so. The notes are the release
// feed's (packaging/notes.md when it was published), kept by ui/Update.tsx;
// their markdown is drawn as text: headings, bullets and paragraphs.
import { keymap, Text, View } from "nsh";
import { tr } from "./i18n";
import { question } from "./model";
import { Badge, Modal, TextButton } from "./parts";
import { reading } from "./Update";

type Block = { kind: "heading" | "bullet" | "text"; text: string };

/** The notes' lines as blocks; marks inside a line (bold, code) are dropped, not drawn. */
function blocks(notes: string): Block[] {
  const plain = (text: string) => text.replace(/\*\*|`/g, "").trim();
  const out: Block[] = [];
  for (const line of notes.split("\n")) {
    const text = line.trim();
    if (!text) continue;
    const heading = /^#+\s+(.*)$/.exec(text);
    const bullet = /^[-*]\s+(.*)$/.exec(text);
    if (heading) out.push({ kind: "heading", text: plain(heading[1]) });
    else if (bullet) out.push({ kind: "bullet", text: plain(bullet[1]) });
    else out.push({ kind: "text", text: plain(text) });
  }
  return out;
}

/** Above the window, beside the other modals. */
export function WhatsNewDialog() {
  const close = () => reading.set(null);
  keymap.bind("escape", () => {
    if (question()) return;
    if (reading()) close();
  });
  return (
    <If when={reading()}>
      <Modal
        testID="whats-new"
        width={520}
        title={tr("What's new in %s", reading()?.notes.version ?? "")}
        aside={
          <If when={reading()?.pending}>
            <Badge label={tr("Not installed yet")} tone="accent" testID="whats-new-pending" />
          </If>
        }
        onClose={close}
        foot={
          <>
            <View class="flex-1" />
            <TextButton label={tr("Close")} testID="whats-new-close" onPress={close} />
          </>
        }
      >
        <View class="gap-[7px]">
          <Each of={blocks(reading()?.notes.text ?? "")} key={(_, i) => i}>
            {(b) => (
              <If when={b().kind !== "bullet"} else={
                <View class="flex-row gap-[7px] pl-[2px]">
                  <Text class="text-[12.5px] leading-[19px] text-muted-foreground">•</Text>
                  <Text class="flex-1 text-[12.5px] leading-[19px] text-fg-2">{b().text}</Text>
                </View>
              }>
                <Text class={b().kind === "heading" ? "pt-[5px] text-[12.5px] font-semibold text-foreground" : "text-[12.5px] leading-[19px] text-fg-2"}>{b().text}</Text>
              </If>
            )}
          </Each>
        </View>
      </Modal>
    </If>
  );
}
