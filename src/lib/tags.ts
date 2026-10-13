// Hashtag helpers shared by the list and the editor. They mirror the Rust
// rules in `categorize.rs`: a tag is `#` plus word characters, `-` or `:`,
// at the start of the text or after whitespace.

const TAG = /(^|\s)#([\p{L}\p{N}_:-]+)/gu;

export function hashtags(text: string): string[] {
  return [...text.matchAll(TAG)].map((m) => m[2].toLowerCase());
}

/** Removes the hashtags `isTag` accepts, tidying the whitespace they leave behind. */
export function stripTags(text: string, isTag: (tag: string) => boolean): string {
  return text
    .split("\n")
    .map((line) =>
      line
        .replace(TAG, (m, pre: string, tag: string) => (isTag(tag.toLowerCase()) ? pre : m))
        .replace(/\s{2,}/g, " ")
        .trimEnd(),
    )
    .filter((line, i, lines) => line.trim() || (i > 0 && lines[i - 1].trim()))
    .join("\n")
    .trim();
}
