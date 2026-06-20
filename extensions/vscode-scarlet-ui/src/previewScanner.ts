const PREVIEW_ATTR_REGEX = /#\[\s*scarlet_ui\s*::\s*preview\s*(?:\([^)]*\))?\s*\]/;
const FN_NAME_REGEX = /^\s*(?:pub\s+)?(?:async\s+)?fn\s+(\w+)/;

export interface DetectedPreview {
  functionName: string;
  line: number;
}

export function scanPreviews(text: string): DetectedPreview[] {
  const lines = text.split("\n");
  const results: DetectedPreview[] = [];

  for (let i = 0; i < lines.length; i++) {
    if (!PREVIEW_ATTR_REGEX.test(lines[i])) continue;
    for (let j = i; j < Math.min(i + 3, lines.length); j++) {
      const match = FN_NAME_REGEX.exec(lines[j]);
      if (match) {
        results.push({ functionName: match[1], line: j });
        break;
      }
    }
  }
  return results;
}

export function toPreviewDisplayName(fnName: string): string {
  return fnName
    .split("_")
    .filter((w) => w.length > 0)
    .map((w) => w.charAt(0).toUpperCase() + w.slice(1))
    .join(" ");
}
