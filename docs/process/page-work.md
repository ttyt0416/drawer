# Page work contract

A local .drawer-work manifest stores512 or fewer saved native-page paths, stable page IDs, a monotonic ID allocator, binding and initial side. Existing ID-less manifests remain readable. The page model keeps up to128 order operations under an8MiB history budget. IDs from an abandoned undo branch are not reused. Removing a page changes only membership: source files and open documents remain. The final page is retained; empty native pages are allowed.

Opening a manifest performs no native page loads. Navigation, facing-index calculation, reorder and delete operate on metadata. The UI virtualizes the list, caches a bounded number of previews and loads only the current/facing pages. Missing pages produce errors without changing their references. Open page IDs map to existing document IDs, and an additional view reuses that document. The workspace view cap remains256. Pending path activation cannot silently bind an unrelated drawing to a page ID.

Left/right panels reserve canvas space; floating placement, size, list offset, automatic following, new-view choice, current page and last saved project are local workspace data. Scroll-following never changes page order, document state or pixel history. Automatic following off preserves the offset; enabling it brings the selected row back into view. Closing the manager retains the model and open drawings.

Project save serializes open documents from their current states and copies closed native pages into an exclusively created staging directory. Every copied page is validated, hashed and synchronized; closed sources are rehashed before publication. The directory is renamed to an independent resource set and the manifest is atomically replaced last. A failure/cancellation removes only the invocation-owned directory and leaves original pages, the previous manifest, foreign temporary files, live views, history and individual save state unchanged. Previously published snapshots are persistent project resources and are not treated as temporary files. Snapshot resources are bounded to2GiB; page decoding and open-document snapshots retain the ordinary document memory limits.

Native page format staysv93. The work manifest adds optional stable-ID metadata to its existing local schema and rejects unknown or invalid fields. The story editor shares the512-page manifest limit while retaining its total memory/row limits.

Integration with webtoon edits also verifies frames containing shared animation tracks. A frame family applies each frame’s own strip mask to its child coverage before union, so shared sources retain every surviving row across a blank-band insertion.

The story editor resolves manifest paths through the current project’s stable page-to-document map. A saved snapshot therefore retains the live document identity even while the original page’s individual path and save state are preserved.
