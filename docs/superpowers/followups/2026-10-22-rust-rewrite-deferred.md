# Rust rewrite: deferred findings

Minor findings from the per-task reviews of the rust-rewrite branch (2026-10-22), none blocking merge. The whole-branch review triaged them: everything listed here can wait for follow-up work. Items already fixed in the final fix wave are omitted.

- **Task 1**: LOG_LEVEL bad value silently falls back to info; no tests for load() missing-token failure or invalid values.
- **Task 2**: finish/fail lack state='running' guard (cancel can be overwritten); list runs one tag query per row; idx_media_slug duplicates UNIQUE index; open_memory pool idle_timeout; no build.rs rerun-if-changed for migrations; tags create/rename duplicate errors opaque (no 409 mapping); rendition.state has no CHECK.
- **Task 4**: multi-key persist not atomic; no semantic validation (ladder [], workers 0, upload_folder with ".." — relevant to Task 6 path safety); Host used verbatim when public_base_url unset; public_base_url not trimmed/validated; update test does not assert DB empty after failed patch.
- **Task 4**: malformed instance_secret still aborts load; concurrent test asserts only value prefixes.
- **Task 6**: Multipart extractor runs before auth (unauth non-multipart POST → 400 not 401); post-rename failure can orphan final file; clash-suffixed titles include -2; no sweep of stale .incoming-* after crash; 413 sent without draining body.
- **Task 7**: roots canonicalised per request; canonicalize→open symlink swap race; `+480p.mp4` parses as alias; no rendition happy-path or HEAD test.
- **Task 9**: form content-type match case-sensitive; login has no CSRF; session ids stored unhashed; no-password path timing differs.
- **Task 9**: blocked_ip_does_not_consume_global_budget test is vacuous (single client); raw space in next rejected (fine).
- **Task 10**: invalid delete_file query → axum plain 400 before CSRF; tag rename race → 500 not 409; tag patch/delete use list_all for existence check.
- **Task 10**: remove_file(src) failure after hard_link leaves a stale duplicate (warn only); check-then-mkdir symlink TOCTOU on an admin-only route; hard_link fallback copy path untested.
- **Task 11**: $$(".sel") also matches tr.sel; stats counts ignore q filter; error toast lost on reload after partial bulk failure.
- **Task 12**: URL settings only prefix-checked (userinfo/whitespace accepted); ?error= text reflected escaped; no gate next round-trip test.
- **Task 13**: revoke loads whole list for existence check; hash-leak test asserts on the substring "hash".
- **Task 13**: success-handler exception would leave the Create button disabled; :has() needs a modern browser.
- **Task 14**: Windows PATHEXT ignored; run never emits final 1.0 on progress=end and fires twice per block (us+ms); encoder not re-detected on settings change (Task 17 handles); check creates data dir/db as side effect; clip.rs normalise_container redundant.
- **Task 15**: fail/requeue results ignored in settle (event may be wrong in a narrow cancel window); no tests for the panic path or image limits; recovered jobs keep attempts.
- **Task 16**: watcher channel unbounded; watcher roots fixed at startup; symlinked external trees walked before rejection; duplicate/non-media files re-hashed every pass; empty mount dir counts as existing root; interval change applies after current sleep.
- **Task 17**: last progress updates may be dropped before ready; tmp name hardcoded twice (move to DerivedPaths); clip-page progress static until reload; view() shows stale encoder during redetect.
- **Task 17**: cancel test should force ENCODER=x264 to avoid hardware-encoder flakiness; rename failure leaves tmp + running rendition until give-up; Unavailable path emits failed then done events; Transcode-all uses bulk so no skipped toast.
- **Task 18**: ImagePage carries dead placeholder fields; og_mime falls back to image/png; image_response duplicates ~40 lines of clip_page; no anonymous-visitor assertion that data-csrf/admin.js are absent on the image page.
- **Task 19**: rendition length matched by label string; no test for rendition-as-enclosure or escaped titles; `let _ = id;` leftover in tests/feed.rs.
- **Task 20**: claim leaks if payload build errors after claim; PATCH announces before tag update (stale tags); bulk public spawns unbounded sends; link-only test proceeds silently if thumb never appears.
- **Task 21**: clash-suffixed title; copy fallback not atomic; import_if_needed/import double-scan; duplicate-path remove_file errors discarded.
- **Task 21**: files_route_rejects_bad_ids lacks the 24-char %2F variant (decoding verified by reading); find_orphan is O(pairs×files); duplicate-content counted as skipped; sweep could delete a concurrent CLI import's temp.
- **Task 22**: push trigger has no branch filter (double runs); no Docker layer/gha caching; no HEALTHCHECK --start-period; musl note misplaced in README; ghcr placeholder owner; TRUSTED_PROXIES bridge-gateway claim untested; release has no test gate.

Also noted by the whole-branch review (post-merge candidates): allow startup without UPLOAD_TOKEN when an admin password is set; delete files under uploads/ regardless of source; sweep stale .incoming-* temps at startup; empty-but-existing scan root marks rows unavailable; X-Robots-Tag noindex on link-only pages; graceful shutdown, compression and access logs; encoder re-detect generation counter; Windows \\?\ canonical path smoke test; webhook client redirect policy none; ffmpeg -protocol_whitelist; duplicate #[cfg(unix)] in tests/media_files.rs.
