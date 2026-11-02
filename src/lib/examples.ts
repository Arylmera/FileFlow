// Worked examples shown under template fields, so the user sees what a
// layout/rename/album template produces before saving it.

/** A worked example of the folder a layout template produces. */
// Shared sample values for the capture tokens every template accepts.
const capture = (s: string) =>
  s.replace(/\{time\}/g, "140322").replace(/\{camera\}/g, "ILCE-7M4");

export function layoutExample(template: string): string {
  const folder = capture(template)
    .replace(/\{year\}/g, "2026")
    .replace(/\{date\}/g, "2026-06-20")
    .replace(/\{name\}/g, "Holiday")
    .split("/")
    .map((s) => s.trim())
    .filter(Boolean)
    .join("/");
  return `${folder}/DSC0001.ARW`;
}

/** A worked example of where a folder-move rule lands a file. */
export function folderExample(template: string): string {
  const folder = capture(template)
    .replace(/\{year\}/g, "2026")
    .replace(/\{date\}/g, "2026-06-20")
    .replace(/\{name\}/g, "")
    .split("/")
    .map((s) => s.trim())
    .filter(Boolean)
    .join("/");
  return folder ? `${folder}/file.jpg` : "file.jpg";
}

/** A worked example of the filename a rename template produces (extension always kept). */
export function filenameExample(template: string): string {
  if (!template.trim()) return "DSC0001.ARW (unchanged)";
  const stem =
    capture(template)
      .replace(/\{orig\}/g, "DSC0001")
      .replace(/\{year\}/g, "2026")
      .replace(/\{date\}/g, "2026-06-20")
      .replace(/\{name\}/g, "Holiday")
      .replace(/\{seq\}/g, "0001")
      .replace(/\//g, "-")
      .trim() || "0001";
  return `${stem}.ARW`;
}

/** A worked example of the album name a date template produces. */
export function albumExample(template: string, name = ""): string {
  const rendered = capture(template)
    .replace(/\{year\}/g, "2026")
    .replace(/\{date\}/g, "2026-06-20")
    .replace(/\{name\}/g, name)
    .split("/")
    .map((s) => s.trim())
    .filter(Boolean)
    .join("/");
  return rendered || "Imported";
}
