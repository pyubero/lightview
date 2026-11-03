# lightview — Group Duplicates (design)

*Status: built on branch `group-duplicates`, 2026-11-03. Describes how the tool behaves. `ROADMAP-v3.md` lists "duplicate finding" as a non-goal; the user is open to reopening it (2026-11-03).*

## In one paragraph
Tools → **Group Duplicates in Folder…** hashes every image in the current folder with a perceptual hash (pHash), shows how close the images are to each other and suggests a threshold, lets the user move a slider while watching the groups change, and then moves each group of look-alikes into a subfolder named after the group's first image. Nothing is deleted, nothing is written except those folders, nothing is cached, and one Ctrl+Z puts every file back.

## Name
"Deduplicate" suggests deleting. The tool only *groups*, so the menu says **Group Duplicates in Folder…**. Same menu as Read QR Code: Tools, since it needs no AI download.

## Scope
- **Folder:** the shown image's folder, the same list next/prev uses (`files::list_folder`, current sort order). Subfolders are not entered; hidden files and unsupported types are skipped, as in navigation.
- **Formats:** everything `load` opens, through `load`'s own path, so the same size limits and guards apply. Animations and multi-page TIFFs use their first frame; RAW its embedded preview; SVG a small render.
- **HEIC/AVIF:** included (user's choice), through the same libheif/WIC path as the viewer. They're never decoded ahead while viewing; here the user picked the folder, and phone folders are mostly HEIC. Where libheif isn't installed (or Windows lacks the codec) they count as "not compared".
- **Files that fail to decode** are counted as "not compared", listed by name, and never moved.
- Needs at least 2 images; otherwise the menu item is disabled (`App::enabled`).
- If the shown image has unsaved edits, the usual "unsaved edits" question comes first, since the viewer may move on to another image (see The viewer afterwards).

## Hashing
- **Input:** a small decode, not the full image. JPEGs use libjpeg-turbo's 1/8 decode (12 MP → 500×375); other formats decode normally and are resized. EXIF orientation is applied first, so the hash matches what the user sees. The EXIF thumbnail is *not* used: it can be stale after editing elsewhere, and some cameras pad it with black bars.
- **pHash:** greyscale → 32×32 (RGB-as-grey, `fast_image_resize`) → 2-D DCT → the 8×8 lowest frequencies → each bit = coefficient above their median (as the common `imagehash` pHash). 64 bits per image. About 40 lines; no new crate.
- **Distance:** Hamming distance between two hashes (`(a ^ b).count_ones()`), 0–64. Unrelated photos land around 28–36; a resized or recompressed copy is usually ≤ 4.
- **Threads:** its own small pool (the decode pool's 3 workers), separate from `Loader`, so viewing stays responsive. Images are hashed in folder order; each result goes back over the existing message channel.
- **Time:** dominated by reading and decoding. Measured 10.9 ms per image (wall time, 3 threads) on 30 generated 12 MP JPEGs with libjpeg-turbo, so 1 000 photos ≈ 11 s; real photos (more detail, bigger files, a cold disk) will be slower; PNGs and HEICs are slower (full decode). Pairwise distances are cheap: 10 000 images = 50 M popcounts ≈ 50 ms.

## Grouping
Connected components (single linkage), user's choice:

1. Two images are **linked** when their distance is within the threshold.
2. A group is every image reachable through links: if A is close to B and B to C, A, B and C are one group, even when A and C are far apart.
3. Groups smaller than **Smallest group** (2 by default, so never groups of one) aren't groups: those images stay where they are.
4. The group's **first image** is its first member in folder order (the current sort order); the group's folder is named after it.

Why: a 30-shot burst drifts a little from shot to shot, so the first and last frames can differ a lot, yet they're one moment and belong in one folder. Chaining through neighbours keeps the whole burst together.

The cost of chaining: a slowly changing sequence (a timelapse, a pan across a landscape, many screenshots of the same app) can join into one large group. The group list shows the size of each group before anything moves, and lowering the slider splits it.

Implementation: union-find over all pairs within the threshold; 10 000 images = 50 M comparisons ≈ 50 ms, so regrouping on every slider move stays instant.

## Choosing the threshold
After hashing, the dialog shows:

- **A histogram** of each image's *nearest-neighbour* distance (the closest other image in the folder), bars 0–32, one bar for "> 32". Duplicates make a pile near 0; unrelated images a hump around 30. The gap between them is where the threshold belongs.
- **A slider**, 0–20 bits, with a marker on the suggested value and the histogram's bars below the threshold highlighted. Words under it: 0 = identical pixels, 1–4 = resized or recompressed copies, 5–10 = small edits, crops or bursts, above 10 = merely similar scenes.
- **The suggested value:** Otsu's threshold over the nearest-neighbour distances, clamped to 2–12. With fewer than 2 distances under 12 (no visible duplicate pile), it suggests 6. The slider starts there.
- **A live summary:** "47 groups · 133 images will move · 1 071 stay · 3 not compared".
- **The groups**, scrollable: the folder name each will get, then its members in folder order as small thumbnails (96 px, kept from the hashing decode) with file names and each one's distance to its closest other member. The largest groups come first, since that's where chaining shows. Clicking a thumbnail shows that image in the viewer behind the dialog. This is what catches a bad threshold (screenshots of the same app, two sunsets).

- **Smallest group**, a second slider (2–50, starts at 2): groups with fewer images aren't shown or moved. A pair of copies may not be worth a folder when a 30-shot burst is; this moves only the big groups. The summary, the list and Move follow it; the histogram doesn't (it shows distances, not groups).

The sliders only regroup (instant); they never re-hash. Both last only while the dialog is open.

## Moving
On **Move**:

- Each group gets a new folder inside the current one, named after the group's first image's file name **without its extension** (`IMG_0412.jpg` → `IMG_0412/`). If that name is already taken by any file or folder, add ` (2)`, ` (3)`… The first image moves in too (user's choice): the whole group goes in.
- Files move with `fs::rename` (same folder tree, so the same filesystem, so atomic). The target is checked first, since `rename` silently replaces (the same gotcha as `load::rename`). A file that vanished or changed since hashing (`files::stamp` differs) is skipped and reported, not moved.
- An error stops the run; what already moved stays moved and is still undoable; the dialog says which file failed and why.
- **Result line:** "Moved 133 images into 47 folders." plus any skipped files.

## Undo
The moves (from → to) and the folders created are kept in memory for the session. Ctrl+Z (after edit undo, as with Trash) moves every file back and removes the folders it created **only if they are empty**. A file whose old name was taken in the meantime is left where it is and reported. Undo ends with the session; that's the point of not writing a journal to disk.

## The viewer afterwards
The viewer stays in the original folder (user's choice).
- The moved files are dropped from the folder list and their cached decodes freed, as `App::delete_image` does after Trash, then the folder is re-listed (`load::relist`). Next/prev only shows the images left in the folder.
- If the shown image moved, the viewer shows the next image left in the folder (the previous one if it was the last), exactly as after deleting the shown image. If no image is left, it behaves as after deleting the folder's last image.
- Open Recent entries pointing at moved files are updated to their new paths.

## No cache
Nothing is stored on disk: no hash database, no thumbnail cache, no file in the folder other than the group folders. Hashes, thumbnails and their textures live in the dialog's state and are dropped (textures freed) when the dialog closes, whether by Move, Cancel or Escape, and when hashing is cancelled. A second run hashes everything again, so it reflects the folder as it is then.

One thing lightview won't control: the operating system keeps recently read files in RAM (the page cache), so a second run right after the first reads from memory and is faster. Dropping that needs root or per-file calls that would also evict files other apps are using, and a cold run isn't more correct, only slower. Benchmarks should time the first run after a reboot, or with `echo 3 | sudo tee /proc/sys/vm/drop_caches`.

## Cancelling
The hashing screen shows "Comparing images: 312 of 1 204" with a progress bar, the time left at the pace so far ("About 40 s left", "About 3 min left", shown once 3 images and a second have gone by) and Cancel. Cancel stops the workers after their current file and drops everything. Nothing has moved until the Move button, so cancelling at any point before it leaves the folder untouched.

## Limits (for `docs/FEATURES.md` and the menu hover hint)
- Rotated or mirrored copies aren't found (pHash isn't rotation-invariant), unless the rotation is only an EXIF flag.
- Heavy crops (more than ~10–15 %) and big edits look like different images.
- Very plain images (blank pages, all-black frames) all hash alike and may group together; the group list shows it before anything moves.
- One folder at a time; no subfolders, no comparing two folders.
- Chaining (see Grouping) can merge a slowly changing sequence into one large group; lower the threshold to split it.
- Sidecar files (`.xmp`, `.aae`) stay behind; a RAW and its JPEG of the same shot will usually land in the same group, which is fine.
- Undo lasts only for the session.

## Where the code goes
- `src/dedupe.rs`: `phash`, `group(hashes, threshold)`, `suggest(nearest)`, `move_groups` and `undo`. Unit tests: a resized + recompressed copy is within 4; two different test images are over 20; grouping on hand-made hashes (a chain A–B–C with A and C far apart forms one group; the group's first image follows the sort order); `suggest` on a bimodal and a flat histogram; moving with a name collision and an existing target; undo restoring names and removing only empty folders.
- `src/app/dialogs.rs`: the hashing, review and result screens.
- `src/app/menu.rs`: the Tools item, a `menu::bindable()` line, `App::enabled` arm.
- `locales/*.tsv`: every new string, in all locales.
- `docs/FEATURES.md`, `docs/MENUS.md`: the feature and its limits.

## Done when
- A test folder (originals, resized copies, recompressed copies, a burst, unrelated photos, a HEIC copy of a JPEG) groups as expected at the suggested threshold, with the whole burst in one folder.
- 1 000 12 MP JPEGs hash in under 10 s on this machine (turbo off counts as pessimistic already).
- Cancel, Move, and Ctrl+Z each leave the folder exactly as described above, checked with a test that lists the folder before and after.
- After Move with the shown image in a group, the viewer shows the next image left in the original folder.
- Running it twice in a row gives the same groups, and the dialog holds no memory once closed.

## Settled (user's choices, 2026-11-03)
1. The v3 non-goal "duplicate finding" may be reopened; this document describes the behaviour.
2. HEIC/AVIF are included.
3. The group's first image moves into the group folder with the rest.
4. When the shown image moves, the viewer stays in the original folder.
5. Grouping chains through neighbours, so a burst stays in one folder.
6. Menu name: Tools → Group Duplicates in Folder….
