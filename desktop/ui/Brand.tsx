// pansy's mark and name, where magpie has its brand: the sidebar's top, or
// the title bar's left end while the sidebar is hidden. As magpie's bird
// flaps, flicks its tail and bobs (app.css .logo.wag), the pansy's petals
// flutter about its heart and the flower nods: as the window opens and when
// the pointer comes over it, not again while it moves, and not at all for
// less motion. Each petal is drawn alone in the mark's box (ui/icons'
// pansy-left, -bottom and -right), so at rest they are the mark whole.
import { createMotion, reduceMotion, Text, View, type MotionSpec } from "nsh";
import { Icon, type IconName } from "./parts";

/** Where the petals meet, in the mark's box: what they and the flower turn about. */
export const HEART = "49.9% 46.4%";

/** magpie's wag: 0.9 s, quick away and slow to settle. */
export const WAG = { duration: 900, easing: [0.3, 0, 0.3, 1] as [number, number, number, number] };

/** The flower nods, a little less each way. */
const NOD: MotionSpec = { ...WAG, keyframes: { rotate: [0, -6, 3, -1.5, 0] } };

/** Each petal's flutter: the upper two open and close as a pair, the lower rocks after them. */
const PETALS: { name: IconName; delay: number; flutter: MotionSpec }[] = [
  { name: "pansy-left", delay: 0, flutter: { ...WAG, keyframes: { rotate: [0, -14, 6, -7, 2, 0], scale: [1, 1.06, 0.98, 1.02, 1, 1] } } },
  { name: "pansy-right", delay: 40, flutter: { ...WAG, keyframes: { rotate: [0, 14, -6, 7, -2, 0], scale: [1, 1.06, 0.98, 1.02, 1, 1] } } },
  { name: "pansy-bottom", delay: 90, flutter: { ...WAG, keyframes: { rotate: [0, 9, -6, 3, -1, 0], scale: [1, 1.05, 0.98, 1.01, 1, 1] } } },
];

/** It wags as the window first opens, once: drawn again (the sidebar shown or hidden), it wags on the pointer only. */
let opened = false;

/** The mark and "pansy"; the mark wags as the window opens and when the pointer comes over either. */
export function Brand() {
  const flower = createMotion();
  const petals = PETALS.map(() => createMotion());
  let moving = false;
  const wag = () => {
    if (moving || reduceMotion()) return;
    moving = true;
    const done = [flower.play(NOD), ...petals.map((m, i) => m.play(PETALS[i].flutter, { delay: PETALS[i].delay }))];
    void Promise.all(done).finally(() => (moving = false));
  };
  if (!opened) {
    opened = true;
    setTimeout(wag, 250);
  }
  return (
    <View testID="brand" onMouseEnter={wag} class="flex-row items-center gap-[6px]">
      <View testID="brand-mark" style={{ width: 18, height: 18, transformOrigin: HEART, ...flower.style }}>
        {PETALS.map((p, i) => (
          <View class="absolute left-0 top-0" style={{ transformOrigin: HEART, ...petals[i].style }}>
            <Icon name={p.name} color="foreground" size={18} />
          </View>
        ))}
      </View>
      <Text class="text-[13.5px] font-semibold text-foreground">pansy</Text>
    </View>
  );
}
