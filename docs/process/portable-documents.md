# Portable local documents and resources

Use **Local templates and resource bundles** to export a new directory. The source document and its links remain unchanged. The native file carries editable layers, material pixels, imported model geometry/textures/usage metadata, audio, Unicode text, mixed font mappings, ruby, layout generations and the last confirmed glyph appearance. This is a document/resource bundle, not a redistribution of original applications or pretrained inference models.

Choose either policy:

- Keep original links: the native snapshot keeps the last complete image cache. Opening a bundle does not probe or refresh original files. Import explains that source links remain and that local relinking is explicit.
- **Portable: copy linked image caches**: version 2 writes `linked-000.png` etc beside `template.drawer`, `masks.json` and `manifest.json`. The manifest uses fixed relative names and stable target IDs. Import validates every hash/size, checks each PNG against the native cache without decoding untrusted PNG data, then rebinds links to the moved bundle. The original source selection becomes the captured whole image; the source document keeps its original selections/links. Version 1 bundles remain readable.

Native 126 stores the confirmed glyph appearance. Versions 1–125 lack this snapshot; restore the original font before capturing such text for export, or explicitly review a replacement. See [confirmed text appearance](confirmed-text-appearance.md).

No font file is distributed by this workflow. A missing font is identified by its original name; text and appearance are preserved. Reading/copying text remains available; choose and review a local replacement before content/layout editing. Font installation elsewhere requires its existing explicit copy permission. For exchange formats, review outline/raster conversion and editing losses. Opening does not silently reflow older layout generations.

Limits: native 64 MiB, mask definitions 8 MiB, linked PNGs 16 MiB each/64 MiB total, at most 256 linked caches, manifest 64 KiB, plus the available workspace memory budget. The exporter never overwrites an existing directory. Unknown paths/files, reparse points, changed or missing resources, duplicate target IDs and inconsistent native/PNG caches are refused. Publication is manifest-last in an owned staging directory followed by one rename; cancellation leaves the source unchanged.

The local verification fixture combines a content-identified local font, Korean/Latin mixed mappings and ruby, a linked page and a textured VRM model. It moves the bundle to a Unicode directory, deletes the source font/page/model, invalidates font data, creates an empty profile and compares complete semantic state and rendered pixels. It verifies missing-font guidance, retained-link policy, changed/rehashed PNG refusal and native save/reopen. The same fixture runs in the packaged application. Editor regression tests replay the import/review/discard/adopt controls; footer buttons stay outside the resource scroll area.

This is a relocated-directory/empty-profile test on the current Windows host. A transfer to a separate offline PC remains pending with item 272; it is not represented as physical cross-PC evidence.
