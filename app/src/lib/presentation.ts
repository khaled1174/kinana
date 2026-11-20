export function skillCount(count: number, locale: string) {
  const form = new Intl.PluralRules(locale).select(count);
  const number = new Intl.NumberFormat(locale).format(count);
  if (locale === "ar") {
    if (form === "one") return "مهارة واحدة";
    if (form === "two") return "مهارتان";
    return `${number} ${form === "few" ? "مهارات" : "مهارة"}`;
  }
  return `${number} ${form === "one" ? "skill" : "skills"}`;
}

export type DiffPart = { text: string; changed: boolean };
// Hirschberg's word alignment keeps memory linear in description length.
// Segmenter preserves Arabic, punctuation and every original whitespace byte.
export function descriptionDiff(original: string, proposed: string) {
  const segmenter = new Intl.Segmenter(undefined, { granularity: "word" });
  const words = (text: string) => Array.from(segmenter.segment(text), s => s.segment);
  const a = words(original), b = words(proposed);
  const keptA = new Set<number>(), keptB = new Set<number>();
  function lengths(x: string[], y: string[]) {
    let row = new Uint32Array(y.length + 1);
    for (const word of x) {
      const next = new Uint32Array(y.length + 1);
      for (let j = 0; j < y.length; j++) next[j + 1] = word === y[j] ? row[j] + 1 : Math.max(next[j], row[j + 1]);
      row = next;
    }
    return row;
  }
  function align(x: string[], y: string[], ai: number, bi: number) {
    if (!x.length || !y.length) return;
    if (x.length === 1) {
      const j = y.indexOf(x[0]);
      if (j >= 0) { keptA.add(ai); keptB.add(bi + j); }
      return;
    }
    const middle = Math.floor(x.length / 2);
    const left = lengths(x.slice(0, middle), y);
    const right = lengths(x.slice(middle).reverse(), [...y].reverse());
    let split = 0;
    for (let j = 1; j <= y.length; j++) if (left[j] + right[y.length - j] > left[split] + right[y.length - split]) split = j;
    align(x.slice(0, middle), y.slice(0, split), ai, bi);
    align(x.slice(middle), y.slice(split), ai + middle, bi + split);
  }
  align(a, b, 0, 0);
  function parts(tokens: string[], kept: Set<number>) {
    return tokens.reduce<DiffPart[]>((out, text, i) => {
      const changed = !kept.has(i);
      if (out.length && out[out.length - 1].changed === changed) out[out.length - 1].text += text;
      else out.push({ text, changed });
      return out;
    }, []);
  }
  return { original: parts(a, keptA), proposed: parts(b, keptB) };
}
