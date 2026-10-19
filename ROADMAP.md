# DesignCraft roadmap

**Stage: pre-alpha** · next: alpha, ~5–7 h away; beta ~28 points and ~420–750 h beyond that (the core-workflow gate, not the percentage: ready for real work is already ~47%)

> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-10-12 · **Change:** minor (text threads by hand; the alpha gate waits on Primary Text Frame page fixes) · **Target:** Adobe InDesign 2026
> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-10-13 · **Change:** minor (parent canvas refresh) · **Target:** Adobe InDesign 2026
> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-10-13 · **Change:** minor (bounded control click/drag input) · **Target:** Adobe InDesign 2026
> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-10-12 · **Change:** minor (text stroke behind fill) · **Target:** Adobe InDesign 2026
> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-10-13 · **Change:** minor (text threads by hand; the alpha gate waits on Primary Text Frame page fixes; Links source paths) · **Target:** Adobe InDesign 2026
> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-10-13 · **Change:** minor (table anchors keep their own paragraphs) · **Target:** Adobe InDesign 2026

DesignCraft aims at full Adobe InDesign parity, and to be better: faster, open (documented JSON
format + IDML), scriptable by agents (MCP), and available on the web.

## Headline numbers

| | Value | Kind |
|---|---|---|
| **Feature breadth** | **~80%** (69% of InDesign 21.6's in-scope menu items matched; 99% of our 275-row checklist) | measured bounds, estimate between |
| **Ready for real work** | **~47%** (range 42–52%) | estimated: weighted sum over the dimensions below |
| Mainstream practitioner | **~42%** (weekly core 60.7% × 0.88 interaction × 0.92 stability × 0.85 file exchange) | estimated |
| Essentials user | **~54%** (core features 72% × 0.90 launch × 0.88 discoverability × 0.95 files received) | estimated |
| Remaining to **beta** | **420–750 Opus 5.5 agent-hours** (~120–200 wall-clock hours with 4–5 agents) | estimated |
| Remaining to **full parity** | **1,150–1,950 Opus 5.5 agent-hours** | estimated |

| Audience | Ready | Opus 5.5 agent-hours to ~95% | Work that dominates |
|---|---|---|---|
| Full target (ready for real work) | ~47% | 1,150–1,950 (~70% parallel) | depth across all areas, scripting compatibility, localization, typography fidelity, INDD/IDML |
| Mainstream practitioner | ~42% | 500–850 (~65% parallel) | INDD import and IDML corpus, composition fidelity, weekly-area depth, PDF/X-1a and printing |
| Essentials user | ~54% | 135–250 (~50% parallel) | start-up robustness, context menus and copy/paste, threading, fonts, Windows printing |

**Why pre-alpha:** it fails one of InDesign's core workflows, the alpha gate in
[docs/roadmap.md](docs/roadmap.md#alpha-gate). Text threads by hand (click a port, then click or
draw a frame; thread and unthread commands), but Primary Text Frame autoflow adds pages users
can't delete (#165). Everything else in the gate works end to end on macOS (document setup,
styles, images, object editing, PDF export, save and reopen), and ready for real work (~47%) is
above the alpha bar, so the Primary Text Frame page fixes are the whole distance to alpha.
Beyond that, beta needs INDD opening, IDML proven on real files, composition compared with
InDesign's, and basic editing fixed on Windows and Linux. Details:
[docs/target-app-parity.md](docs/target-app-parity.md).

## By dimension

| Dimension | Ready | Hours | Doc |
|---|---|---|---|
| Features (depth) | 55% | 250–400 | [target-app-parity.md](docs/target-app-parity.md) |
| Typography and composition fidelity | 45% | 120–200 | [typography-parity.md](docs/typography-parity.md) |
| File formats | 40% | 150–250 + 20–60 (INDD) | [file-format-parity.md](docs/file-format-parity.md) |
| UI/UX fidelity | 45% | 120–180 | [ui-parity.md](docs/ui-parity.md) |
| Stability | 45% | 60–100 | [gaps.md](docs/gaps.md) |
| Performance | 65% | 40–80 | [gaps.md](docs/gaps.md) |
| Localization | 45% | 120–180 | [localization-parity.md](docs/localization-parity.md) |
| Platforms | 55% | 40–80 | [hardware-parity.md](docs/hardware-parity.md) |
| Hardware | 45% | 20–40 | [hardware-parity.md](docs/hardware-parity.md) |
| Ecosystem (scripting, plug-ins) | 15% | 150–250 | [gaps.md](docs/gaps.md) |
| AI features | 15% | 60–120 | [gaps.md](docs/gaps.md) |

All "Ready" figures are estimates; the weights and evidence are in target-app-parity.md.

## Features

Breadth from [docs/parity-checklist.md](docs/parity-checklist.md) (`cargo xtask parity`,
measured); Ready estimated.

| Area | Breadth | Ready | Hours |
|---|---|---|---|
| Application shell & workspace | 99% | 55% | 40–60 |
| Documents, pages, spreads | 100% | 65% | 20–35 |
| Layers | 100% | 75% | 5–10 |
| Frames, shapes & paths | 100% | 60% | 25–40 |
| Transform | 100% | 70% | 10–20 |
| Fill, stroke, colour | 100% | 60% | 20–35 |
| Effects & transparency | 100% | 50% | 20–30 |
| Placing & links | 93% | 55% | 20–35 |
| Type & text frames | 99% | 45% | 40–60 |
| Typography | 100% | 45% | see typography-parity.md |
| Styles | 100% | 60% | 15–25 |
| Tables | 100% | 55% | 25–40 |
| Long documents | 100% | 45% | 30–50 |
| Interactivity & digital | 100% | 40% | 30–50 |
| Output & production | 92% | 55% | 40–70 |
| XML & automation | 100% | 35% | see Ecosystem |
| View & navigation | 100% | 65% | 10–20 |
| Undo, history, saving | 100% | 70% | 5–10 |
| Accessibility | 100% | 50% | 15–25 |

## Languages

Interface strings translated, of 1,724 measured. Detail: [docs/localization-parity.md](docs/localization-parity.md).

| Language | Code | Status | Translated |
|---|---|---|---|
| English | en | full | 100% |
| Simplified Chinese | zh | partial (no canvas IME) | 83% |
| Spanish | es | partial | 83% |
| Hindi | hi | none | 0% |
| Arabic | ar | partial (RTL UI and composition) | 85% |
| French | fr | partial | 83% |
| Portuguese (Brazil) | pt-br | partial | 61% |
| Indonesian | id | none | 0% |
| Japanese | ja | partial (no canvas IME) | 95% |
| German | de | partial | 83% |
| Korean | ko | none | 0% |
| Vietnamese | vi | none | 0% |

Also shipped: Italian (85%), Ukrainian (83%), Georgian (100%).

## Upcoming

Ranked; detail and estimates in [docs/roadmap.md](docs/roadmap.md), the full list in
[docs/gaps.md](docs/gaps.md).

1. **Alpha gate:** Primary Text Frame page fixes: autoflow adds pages users can't delete (#165) (5–7 h)
2. Basic editing on every platform: copy/paste, context menus, caret formatting (30–50 h)
3. CI running `cargo xtask ci` on pull requests (4–8 h)
4. Open INDD through a clean-room converter, owner decision on #114 (20–60 h)
5. IDML real-file corpus and fixes (60–100 h)
6. Composition oracle against InDesign; optical kerning; overset rule (80–140 h)
7. Start on any GPU; find every installed font (25–50 h)

## Documents

[target-app-parity.md](docs/target-app-parity.md) (assessment) · [gaps.md](docs/gaps.md) (work
list) · [roadmap.md](docs/roadmap.md) (milestones, plan) · [parity-checklist.md](docs/parity-checklist.md)
(presence checklist) · [typography-parity.md](docs/typography-parity.md) ·
[file-format-parity.md](docs/file-format-parity.md) · [ui-parity.md](docs/ui-parity.md) ·
[hardware-parity.md](docs/hardware-parity.md) · [localization-parity.md](docs/localization-parity.md) ·
[architecture.md](docs/architecture.md) · agents: [agents.md](docs/agents.md), [mcp.md](docs/mcp.md),
[control-protocol.md](docs/control-protocol.md) · CJK: [cjk.md](docs/cjk.md),
[cjk-typography.md](docs/cjk-typography.md) · Arabic: [arabic-typography.md](docs/arabic-typography.md)

## Progress log

- **2026-10-13** · Switching between document and parent pages refreshes the canvas immediately, preventing black parent pages and stale document objects when the zoom stays the same. Ordinary edits retain partial repainting. No readiness percentage change.

Newest first.

- **2026-10-14** · The Type tool draws a new text frame over a locked empty frame, as InDesign does, instead of turning the locked frame into a text frame. No readiness percentage change.
- **2026-10-14** · 100% black shows as pure black on screen by default, as InDesign's Appearance of Black does out of the box (Display All Blacks as Rich Black); Preferences › Appearance of Black › Display All Blacks Accurately still shows it as the dark grey it prints, and a saved choice is kept. No readiness percentage change.
- **2026-10-14** · Pathfinder keeps the frontmost object, as InDesign does: a photo placed over shapes and intersected with them shows through the shapes (it was dropped, and the shapes kept their black fill); Add, Exclude Overlap and Minus Back take the front object's fill too, Subtract still keeps the back one. No readiness percentage change.
- **2026-10-14** · The Color panel's Tint field keeps what is typed while it has focus and applies it on Enter or when it loses focus, clamped to 0–100 %. Typing 60 over 100 % no longer snaps back to 100 after the first digit (10060), and a typed tint is no longer ignored. No readiness percentage change.
- **2026-10-15** · IDML: ruler guides import and export (they were dropped). Each page's guides come in measured from the document's ruler origin (spread, page or spine), with Fit to Page, Locked and their layer, parent pages' included, and go out again under their pages. No readiness percentage change.
- **2026-10-15** · Find Font no longer reports a style missing when it differs from the installed one only in spacing or case (InDesign files can name `Regular ` with a trailing space); the text was already drawn in the right face. No readiness percentage change.
- **2026-10-15** · A line of text is set when its baseline fits in the frame, as in InDesign: descenders may hang below the frame's bottom. A frame sized to the cap height of a big heading (103 pt in a 2.77 cm frame, as Zero2Graphic's magazine tutorial draws them) no longer oversets. No readiness percentage change.
- **2026-10-15** · A story that ends with a return is no longer overset when only its empty last paragraph doesn't fit, as in InDesign (Preflight said "Overset text: 0 characters" for a caption InDesign shows with no errors). No readiness percentage change.
- **2026-10-13** · Object › Fitting has InDesign's shortcuts: Fill Frame Proportionally ⌘⌥⇧C, Fit Content Proportionally ⌘⌥⇧E, Fit Frame to Content ⌘⌥C, Fit Content to Frame ⌘⌥E, Center Content ⌘⇧E. No readiness percentage change.
- **2026-10-13** · Gradients between process colours blend as inks, as InDesign shows them: magenta to cyan passes through C 50 M 50 (a light lilac) on screen instead of the darker RGB mix, and a midpoint between CMYK or grey stops no longer turns the PDF shading RGB. No readiness percentage change.
- **2026-10-14** · The Swatches panel has InDesign's header: a Fill/Stroke proxy, Formatting Affects Container/Text and the Tint of the colour in front (a tint alone keeps each object's own swatch). Clicking a swatch applies it to the fill or stroke in front (of the text, when formatting affects text), and the bottom row has New Color Group, New Swatch and Delete Swatch. No readiness percentage change.
- **2026-10-14** · With the Page tool, the Control panel shows the page as InDesign's does: W/H from a reference point, page-size preset (US Business Card added), orientation, Liquid Page Rule (now with Controlled by Parent) and Show Parent Page Overlay, which draws the parent page's size over a resized page. No readiness percentage change.
- **2026-10-14** · On an object-based page the Page tool shows a selected object's liquid pins and resize locks as InDesign does: a click pins a side to the page edge or locks the width or height, with InDesign's tooltips. No readiness percentage change.
- **2026-10-14** · The Liquid Layout panel follows InDesign's: Liquid Page Rule with the page number, Content › Auto-Fit, and Object Constraints › Resize with Page and Pin for the selected object on an object-based page; Create Alternate Layout moved to the Layout menu. No readiness percentage change.
- **2026-10-14** · With the Page tool, the Control panel shows the page as InDesign's does: W/H from a reference point, page-size preset (US Business Card added), orientation, Liquid Page Rule (now with Controlled by Parent) and Show Parent Page Overlay, which draws the parent page's size over a resized page. No readiness percentage change.
- **2026-10-14** · With the Page tool, the Control panel shows the page as InDesign's does: W/H from a reference point, page-size preset (US Business Card added), orientation, Liquid Page Rule and Show Parent Page Overlay, which draws the parent page's size over a resized page. No readiness percentage change.
- **2026-10-14** · The Page tool works as InDesign's: a click selects a page and shows its handles and liquid page rule; dragging a handle previews the rule and springs back, Alt-drag resizes the page from the opposite edge, Shift keeps proportions, with a W/H readout. No readiness percentage change.
- **2026-10-14** · With the Page tool, a guide dragged from a ruler is a liquid guide (dashed), as in InDesign; guide-based pages stretch the objects it crosses. No readiness percentage change.
- **2026-10-14** · The Pages panel menu follows InDesign's order: Insert Pages…, Move Pages…, Duplicate Spread (and at End of Document), Delete Page/Spread, New Parent…, Apply Parent to Pages, Override All Parent Page Items, Parent Pages, Create Alternate Layout…, Numbering & Section Options…, Allow Selected Spread to Shuffle. No readiness percentage change.
- **2026-10-14** · Pages panel: Alt-drag copies a page, a page dropped on Create new page is duplicated at the end, Edit page size offers the presets and Custom Page Size…; duplicated pages keep their own size, rule and guides; thumbnails of pages of their own size stay in their rows. No readiness percentage change.
- **2026-10-12 (PDFW)** · PDF, booklet and book composition share guarded writing, metadata and font preparation through craft-pdf. Layout, shaping, placed graphics, colour policy, tagging and PDF/X postprocessing remain in DesignCraft. Failed native drawing returns an export error; failed writer cleanup cannot serialize an incomplete document. No readiness percentage change.

- **2026-10-13** · Copy with a placed graphic selected (Direct Selection) copies just the graphic, uncropped, where it sits on the page and without the frame's stroke, so Paste Into puts the whole image in place; Cut and Delete then remove only the graphic and leave its frame empty. Nested selections copy in their spread coordinates. No readiness percentage change.
- **2026-10-13** · Object effects (Basic Feather, Inner Glow, shadows, glows, bevel, satin, directional and gradient feathers) apply to frames with pasted-in content, on screen and in image exports: the frame, its clipped content and its stroke are one object to the effect. PDF export still has no soft effects for any object. No readiness percentage change.
- **2026-10-13** · Links rows show their source paths; selecting a link shows its full path wrapped inside the panel, so same-named files from different folders can be distinguished. No readiness percentage change.
- **2026-10-13** · The Text Wrap panel is laid out as InDesign's: the wrap buttons with Invert, and the four offsets (Top, Left, Bottom, Right) showing the object's values, with the link button that sets them all the same. It used to show a single offset, always 0. No readiness percentage change.
- **2026-10-13** · Radial gradients stretch with the object's box, as InDesign draws them: by default an ellipse reaching the frame's sides (it was a circle reaching only the longer side), and a Gradient Swatch tool drag keeps the box's proportions; on screen and in PDF. No readiness percentage change.
- **2026-10-13** · The Window menu follows InDesign's order (Arrange, Workspace, then the panels alphabetically with Color, Editorial, Object & Layout, Styles… submenus), and Window › Workspace lists the workspaces as InDesign does, now with Essentials Classic: the Control panel and an icon column of named panel groups. No readiness percentage change.
- **2026-10-14** · In a facing document, pages take their side from their section's page numbers, as InDesign shuffles them: a section that starts at 1 on a left page moves it to the right and the pages after it follow, with their objects. No readiness percentage change.
- **2026-10-14** · The Type menu follows InDesign's order (Size, Character ⌘T, Paragraph ⌥⌘T, Glyphs, the style panels, Notes, footnotes and endnotes, then the insert submenus and Show Hidden Characters); Current Page Number has ⌥⇧⌘N. No readiness percentage change.
- **2026-10-14** · Colour parameters: a bare `{c,m,y,k}` is always read as 0..100, so C 1 is a 1 % tint and no longer
  solid cyan; fractions take the colour's own `{model: "cmyk", …}` form (#910). No readiness percentage change.
- **2026-10-14** · The Tools panel follows InDesign's: Gradient Swatch and Gradient Feather are tools of their own; in two columns Content Collector and Placer, Apply Color / Gradient / None and Normal / Preview each get a button; View Options holds Frame Edges, Rulers, Guides, Smart Guides, Baseline Grid and Hidden Characters; Apply Color (,) and Apply Gradient (.) reuse the last colour and gradient. No readiness percentage change.
- **2026-10-12** · Text threads by hand: click a selected text frame's in or out port to load
  the text cursor, then click a frame to thread it, click empty space for a column-wide frame or
  drag one (it snaps like frame drawing); Shift-click autoflows, Alt-click stays loaded on the
  frame just added; click the linked frame or double-click a port to break the thread.
  `text.thread`, `text.unthread`, `text.removeFromThread` with undo; locked frames and layers
  refuse threads. The alpha gate now waits only on the Primary Text Frame page fixes (#165). No
  readiness percentage change.
- **2026-10-14** · Georgian interface (`ka`, Edit ▸ Interface Language ▸ ქართული): every catalog key
  translated, count captions as `noun: n`.
- **2026-10-13** · Layout ▸ Table of Contents opens a real dialog: title and title style, Include Paragraph Styles and Other Styles lists with Add and Remove, and per style its entry style, page number position (after, before, none), separator and level. Replace Existing regenerates the current table of contents in place; otherwise OK makes a new frame on the page in view. IDML export writes the definition as a TOC style that import reads back, so Update works after a round trip. No readiness percentage change.
- **2026-10-13** · Control-channel clicks and drags validate positive counts and the complete pending-event budget before queuing input (#732). Oversized requests leave pending gestures intact, preventing unbounded click-count allocation. No readiness percentage change.
- **2026-10-13** · Backspace between two consecutive tables, pasting a table next to a table anchor, and pasting several paragraphs right before one keep every table on its own anchor paragraph. Previously a table was dropped or left without its anchor, and the saved file failed to reopen. No readiness percentage change.
- **2026-10-13** · Dialogs and the About window move when you drag their title row, so the page behind them can be seen. A dragged dialog may go partly off-screen, but its title row stays reachable; each dialog opens centred, and again after the window is resized. No readiness percentage change.
- **2026-10-13** · Command scripts look up only a well-formed `$N.path` / `$last.path` reference; other strings starting with `$`, such as a GREP replacement of `$1` and a nonbreaking space, stay as written, and `$$` writes a literal `$` (#749). No readiness percentage change.
- **2026-10-13** · An explicit recovery folder (`file.recovery.save` with `dir`) never overwrites or deletes another running app's checkpoints. Document uids start at 1 in every process, so two apps saving recovery to one chosen folder replaced or removed each other's `<uid>.designcraft`. A run replaces or removes only the entries it wrote or recovered there, refuses an occupied uid, and refuses a folder whose run lock another app holds. No readiness percentage change.
- **2026-10-13** · Path edits (`path.setAnchor`, `path.moveAnchors`, `path.appendAnchor`, `path.reverse`, `path.close`, anchor add/delete/convert, Erase, Smooth and Scissors) refuse a path that is locked, sits in a locked group or on a locked layer (#735). Properties links to the Stroke, Character, Paragraph, Effects, Text Wrap and Info panels open them in a customized panel layout too (#736). No readiness percentage change.
- **2026-10-13** · IDML export keeps parent item overrides: each page lists its overridden parent items and the local copy keeps its link to the parent item, so a reopened IDML file shows only the local copy (#725). No readiness percentage change.
- **2026-10-13** · macOS: a command chosen with the mouse in the menu bar runs at once. It used to wait for the next mouse move or key press over the window, so File › Open, Place and Save As seemed to take 10–20 seconds to open their dialogs. No readiness percentage change.
- **2026-10-13** · Text recomposes when the document baseline grid changes, when a layer holding a text-wrap object is shown or hidden, or when a section's start number, numbering style, prefix or marker changes; the composition cache keyed only on the section count and item identities and kept the old layout. No readiness percentage change.
- **2026-10-12** · macOS Dock Quit uses the same unsaved-document prompts as window and menu Quit. Repeated requests keep the current answers; Cancel retains every tab and unsaved recovery checkpoint, and completed Save/Don't Save answers exit through the ordinary event loop. No readiness percentage change.
- **2026-10-12** · Text strokes can sit behind the fill: the stroke lies wholly outside the letters at its full weight on screen, in PNG and in PDF, set from the Properties panel or Character Style Options. IDML `StrokeAlignment` on text imports and exports as this setting, so imported text with outside-aligned strokes keeps its letter shapes (#619). No readiness percentage change.
- **2026-10-12** · Placed Word documents keep their bullets and numbering (#626): `numbering.xml` levels become bullet or numbered paragraphs with Word's number formats, label text, start numbers, restarts and list indents, also when a paragraph style carries the list. No readiness percentage change.
- **2026-10-12** · Orderly native exit writes the latest recovery state after finalizing current quit approvals; incomplete or stale plans keep their documents, and Undo to the saved state clears stale recovery. Unavailable or failed recovery is reported. Cancelable macOS Dock Quit remains open (#544). No readiness percentage change.
- **2026-10-12** · Recovered documents keep a checkpoint until saved, closed or reverted, including a second crash before autosave. Restarted document IDs and explicit recovery folders preserve other snapshots; failed republishing retains the original. No readiness percentage change.
- **2026-10-12** · Book numbering and style updates prepare every chapter before saving. Only closed native chapters are changed; imported IDML remains a read-only style/PDF source. If a later disk write fails, the error names the completed chapters; the batch is not a whole-book transaction. No readiness percentage change.
- **2026-10-12** · Desktop write callbacks and CLI raster exports use the atomic writer too; open readers retain the original bytes, while private Unix modes and symlink targets survive replacement. Permanent tests cover both actual callers. No readiness percentage change.
- **2026-10-12** · Save As, library edits and book collection edits persist prospective values before changing live state; failed writes preserve the document and existing collections. Save a Copy rebases data-source paths for its destination without changing the open document. No readiness percentage change.
- **2026-10-12** · The Pen tool continues an open path from either end point, as in InDesign: the cursor shows it over an end point, a click there carries on drawing that path, a drag first reshapes the end point's direction lines, and the other end closes it. Object › Paths › Reverse Path (`path.reverse`); `path.setAnchor`; `path.appendAnchor` and `path.close` take a subpath. No readiness percentage change.
- **2026-10-12** · Editing type on a path: the Type tool clicks, drags and double-clicks along the curve (the character under the pointer, not one on the straightened line), a click on the letters themselves hits the frame, and the caret and highlight bend with the path. No readiness percentage change.
- **2026-10-12** · Type on a path has start and end brackets, as in InDesign: selected path type shows them across the path, and dragging either with the Selection or Direct Selection tool moves where the text starts or ends (text past the end bracket is overset). `type.pathOptions {end}`; IDML reads and writes EndBracket. No readiness percentage change.
- **2026-10-12** · The Eyedropper puts a colour sampled from an image on the stroke when the toolbox's Stroke box is in front, as in InDesign (a line takes it on its stroke). No readiness percentage change.
- **2026-10-12** · Window close and application/menu Quit ask about unsaved documents; later Cancel keeps every tab open, including earlier Don't Save choices. Approvals are rechecked after edits. Cancelable macOS Dock Quit remains open (#544). No readiness percentage change.
- **2026-10-12** · Each running app keeps recovery data in its own locked folder; current folder and lock ownership are checked before writes and cleanup. Failed isolation disables recovery with a warning instead of sharing document IDs. No readiness percentage change.
- **2026-10-12** · Native engine saves and exports replace files atomically; failed writes retain the original, private staging preserves Unix permissions, and valid long filenames work. Desktop Services and CLI raster exports remain separate follow-ups. No readiness percentage change.
- **2026-10-12** · Hyphenation Zone applies to the Single-line Composer only, as in InDesign. With the default Paragraph Composer, ragged text hyphenates where InDesign does (aper-natem), giving the same line breaks. No readiness percentage change.
- **2026-10-12** · The Tools panel's Fill/Stroke keys work: D (default fill and stroke), X (fill or stroke in front), Shift+X (swap per object, tints included) and / (Apply None), as `color.*` commands; swap and default are one undo step (`object.swapFillStroke`, `object.defaultFillStroke`); the Color panel follows the proxy's fill/stroke choice. No readiness percentage change.
- **2026-10-11** · Corner Options keep curved sides curved and round every corner point, including where an arc meets a line (a Pathfinder quarter circle with rounded corners stayed a triangle). No readiness percentage change.
- **2026-10-11** · `story.replaceRange`, `story.setText`, notes, footnotes, endnotes, type on a path and table cells read `\r` and `\r\n` as paragraph returns, as `frame.create` and `text.insert` already did. No readiness percentage change.
- **2026-10-11** · The View menu follows InDesign's: proofing first, then zoom, Rotate Spread, Screen Mode, Display Performance, rulers, Extras, Grids & Guides and Structure; the view items that sat at the end of the Type menu moved there. No readiness percentage change.
- **2026-10-12** · Underlines and strikethroughs are drawn as one bar per stretch of text on screen, as in PDF export: a bar per letter left faint seams between the letters. No readiness percentage change.
- **2026-10-13** · Placed PDFs are drawn through pdfcraft's patched hayro crates: calculator functions whose `idiv` divides by zero or overflows, chains of stitching functions a hundred thousand deep, and mesh shadings with points far off the page, ten thousand overlapping triangles or forty thousand patches no longer panic, overflow the stack, run for over a minute or use more than 2 GiB of memory. A page with a `/UserUnit`, and its trim, bleed and art boxes, are sized in points. No readiness percentage change.
- **2026-10-12** · Properties › Transform shows rotation, shear and X/Y scale, and Rotate 90° and Flip Horizontal/Vertical buttons, as InDesign's does. No readiness percentage change.
- **2026-10-12** · Edit › Step and Repeat has its InDesign dialog: Repeat (Count, or Rows and Columns with Create as a grid), Offset (Vertical, Horizontal, in ruler units) and Preview, which shows the copies while the dialog is open; Cancel takes them away and OK makes them as one undo step. No readiness percentage change.
- **2026-10-12** · Double-clicking the Fill or Stroke box in the Tools panel opens the Color Picker with the selection's colour, as in InDesign (it opened the Swatches panel). No readiness percentage change.
- **2026-10-12** · The Effects panel shows the level its settings apply to ("Group: Color Dodge 100%") and, as in InDesign, Isolate Blending and Knockout Group for a selected group. No readiness percentage change.
- **2026-10-11** · Properties gains InDesign's Pathfinder section (Add, Subtract, Intersect, Exclude Overlap, Minus Back) with several objects selected; section titles and icon buttons have accessible names. No readiness percentage change.
- **2026-10-12** · The Eyedropper samples a placed image: clicking the photo with text (or objects) selected fills them with the image's colour under the pointer, as an unnamed colour (`color.sample`), as in InDesign. Images on locked layers can be sampled. No readiness percentage change.
- **2026-10-12** · The section marker character shows its section's marker text on document pages (nothing when the section has none) instead of the word "Section", which stays on parent pages; editing the marker recomposes (#556). No readiness percentage change.
- **2026-10-12** · On screen, blend modes blend with the objects below and not with the paper, as in InDesign and in PDF export: a Color Dodge or Screen object over blank paper shows as it is. No readiness percentage change.
- **2026-10-12** · A gradient feather on a group fades the group as one, on screen and in PDF. Before, the screen ignored it, and PDF masked the group with a box of zero size because a group has no path of its own. No readiness percentage change.
- **2026-10-12** · Words in capitals hyphenate like capitalized words (FERRUM COM-MODI), as in InDesign; Hyphenate Capitalized Words off keeps them whole. No readiness percentage change.
- **2026-10-12** · Load Swatches refuses an `.ase` file with more than 100,000 colors, and `swatch.load` merges a file of 100,000 colors in under 0.1 s (it took 9.6 s of CPU in a test build). No readiness percentage change.
- **2026-10-11** · `transform.move {copy}`, Edit › Duplicate and Step and Repeat return the copies' ids, so a script can go on with them. No readiness percentage change.
- **2026-10-11** · `object.fill` and `object.stroke` refuse a swatch name the document doesn't have (it painted nothing) and suggest the name meant (`Paper` → `[Paper]`). No readiness percentage change.
- **2026-10-11 (printing):** `file.printers` and `file.print` use craft-libs/craft-print for CUPS and Windows. DesignCraft retains PDF export, page selection, spreads, marks and bleed; dry runs create no temporary PDF or print job. Windows selects matching or default driver paper and shrinks oversized sheets without enlarging smaller ones; queue duplex defaults are preserved. Headless tests exercise exported bytes, job settings, dry runs and refusals; shared native Windows CI passes, including paused-queue submission and cleanup. PostScript, production print controls and physical output validation remain open. No readiness percentage change.

- **2026-10-10 (UI.4–UI.5):** Panel tabs can move into other groups, split the dock, float, resize and return to their saved locations through shared docking transactions. Layouts persist in custom workspaces; UI and control commands share the same validated action path. Shared pane geometry and widget-state theme application retain all five palettes and local fonts. Split panes send keyboard input only to the focused canvas and scope contextual toolbar IDs per pane. No readiness percentage change.
- **2026-10-10** · Version 0.6.0; releases add a Linux riscv64 tar.gz (cross-compiled, CLI
  smoke-tested under QEMU).
- **2026-10-10** · Readiness per audience: full ~47% (method aligned with the standard, was
  42%), mainstream practitioner ~42%, essentials user ~54%, each with hours to ~95%.
- **2026-10-10** · Stage re-normalized to **pre-alpha** under craftrules' core-workflow gate: text
  can't be threaded by hand (no out-port click, no thread command).
- **2026-10-10** · Full re-measure against InDesign 2026 21.6 (menu dump, bundle formats and
  localizations, 124 issues): breadth ~80%, ready ~42% (down from the ~83% "including depth" of
  2026-10-04, on user evidence and the INDD gap); progress docs restructured to the craftrules
  standard (`docs/parity.md` → `docs/parity-checklist.md`; new target-app-parity, gaps, roadmap,
  architecture, typography, file-format, UI, hardware and localization docs). Landed today:
  split columns (#295), PDF export options dialog (#196), column rules (#274), Paragraph Rules
  dialog (#263), Character Style Options (#144), PDF/X-4 RGB handling (#223), Spanish
  hyphenation (#222), Japanese (#272), Italian (#218) and Ukrainian (#236) interfaces, relocated packages relink (#244), graphics backend chosen
  before the window (#297), IDML table styles (#288), warichu (#280), list numbering from IDML
  (#298), security hardening (#153, #155), many text-composition fixes (#282–#293).
- **2026-10-09** · Version 0.5.0; many user reports triaged into issues (#160–#265).
- **2026-10-08** · Version 0.4.0.
- **2026-10-07** · Community PRs: data merge, Brazilian Portuguese, PDF/X-4 flags, Office fonts
  on macOS; CJK 1.x (Korean line breaking, fallback fonts in Package); PrintCraft renamed PdfCraft.
- **2026-10-06** · Snapping to parent items, Windows packaging, ARM64 workflow, control-channel
  hardening.
- **2026-10-05** · FreeBSD build; caret-boundary fixes.
- **2026-10-04** · Never-crash pass across importers and the engine; rotate spread view; kashida
  in styles; previous estimate: breadth 99%, "~83% including depth", ~170 h remaining.
- **2026-10-03** · Release 0.1.1; separations, XML tagging, track changes, PDF/X-4 output
  intent, split window, buttons, menu customization.
- **2026-10-02** · Hyperlinks/Bookmarks panels, conditional text, tagged PDF, lists across
  stories, style groups, Glyphs panel, High Contrast theme.
- **2026-10-01** · First commit; skeleton, IDML, PDF, tables, footnotes, cross-references, index,
  spelling and parallel composition on the first day.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-12 | minor | Manual text threading landed; alpha distance 15–25 h → 5–7 h (Primary Text Frame page fixes, #165) |
| 2026-10-10 | minor | Full number 42% → 47% (method aligned with the standard: plain weighted sum, no new evidence); readiness table with hours per audience |
| 2026-10-10 | minor | Headline adds mainstream practitioner (~42%) and essentials user (~54%); method in docs/target-app-parity.md |
| 2026-10-10 | minor | Stage alpha → pre-alpha under the new core-workflow gate: manual text threading is missing; alpha gate table added to docs/roadmap.md |
| 2026-10-10 | major | Restructured to the progress-docs standard; full re-measure; the "Working today" inventory moved to docs/target-app-parity.md, milestones to docs/roadmap.md, the CLI/MCP section left to docs/agents.md (which already covered it) |
| 2026-10-04 | major | Parity estimate: breadth 99%, depth ~83%, ~170 h |
| 2026-10-01 | major | First roadmap: status, milestones |
