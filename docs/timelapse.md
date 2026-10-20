# Timelapse (File › Timelapse, Window › Timelapse)

Records a timelapse of your painting while you work and exports it as an MP4. It is the Photoshop
Timelapse Recorder plug-in (a UXP panel) built into PhotoCraft, and it reads and writes that
plug-in's recordings, so a recording started in Photoshop can be continued or exported here.

It is built in rather than a WebAssembly plug-in: recording has to follow every edit, undo and
redo and capture frames in the background, and the plug-in ABI only runs pixel filters.

## The panel

**Window › Timelapse** opens the Timelapse panel, a dock panel like Layers or Brushes: it docks at
the bottom of the panel column (the groups above give way to their compact heights), and it can be
resized by its edges, collapsed, moved, grouped with other panels or floated (panel menu › Float
Panel). It is laid out like the plug-in's panel:

- **Record**: the RECORDING card (Auto record pill; the big record button, which reads Start
  recording, green Recording with a pulsing dot (click to pause) or grey Paused; Export video /
  Export again, which opens the export box with Length, Advanced settings and Confirm; progress
  and Cancel export while exporting; then Play video and Show in folder), the FRAMES / TIMER /
  ON DISK tiles with the document name and Preview ▸, and the REFERENCE LAYER card (the selected
  layer, Normal / Reference).
- **Export settings** (Advanced settings, ‹ Back): VIDEO (length, ffmpeg or JPEG frames, bars
  colour), EXTRAS (finished drawing first and its length, flash, hold and its length, slow down,
  remove flicker, work-time counter and its corner), RESULT (the video's length, frames and size).
- **Library** and **Settings** (frames folder, recording quality Low / Default / High, remove
  frames with undo, the reference tag, the work timer's idle limit, ffmpeg).

Colours come from the theme (`Tokens::success` is the Recording green), so it follows every theme.

## Using it

- **Start recording** (or File › Timelapse › Start Recording) asks for a recordings folder the
  first time.
- A frame is saved shortly after each real edit: 350 ms after the last activity, at most 2.5 s
  after the first edit it covers. Frames are rendered on a worker thread from a copy-on-write
  snapshot, so painting never waits for a capture.
- Not recorded: zoom and other view changes, selecting layers, and showing or hiding layers
  (history steps in PhotoCraft, recognised by name). After a show/hide nothing is captured until
  the next real edit. Frames where (almost) nothing visible changed are skipped.
- **Undo** flags the frames of the undone steps and leaves them out of the video (Settings ›
  Remove frames with undo); redo brings them back; a new edit after an undo drops them for good.
- **Reference layers**: a layer whose name contains the reference tag (`[ref]` by default) stays
  visible to you but is hidden in every frame. File › Timelapse › Reference Layer (or the panel's
  Normal / Reference buttons) adds or removes the tag on the targeted layers.
- **Work timer**: counts time between actions and stops after the idle limit (30 s by default).
- **Auto record** records whatever document is active, unless you paused it yourself. A
  document that was being recorded when it was closed resumes when it is opened again (found by
  the plug-in's recording id in its XMP, or by its path).
- **Linking**: the recording id is written into the document's XMP
  (`http://ns.timelapse-recorder.local/1.0/`, `tlr:RecordingID`, as the plug-in does), so a saved
  file that was renamed or moved still finds its recording. It is written in place, with no
  history step, so a clean document stays clean; it reaches the file with the next save (PSD,
  TIFF, OpenRaster, .pcraft…), and an undo back to before it was written writes it again.
- **Save As** renames the recording and its folder after the document ("Untitled-2 (b26b3312)"
  → "Tree (b26b3312)"), between captures and never while a timelapse is being exported. When the
  folder can't be renamed (that name is taken, or a file in it is open elsewhere) the recording's
  name still changes and the folder is tried again a few seconds later.

## Library

The panel's **Library** tab lists every recording in the folder (newest recording activity
first): frames, size, work time, REC / EXPORTED / NOT EXPORTED, and whether more was recorded
since the last export. **Open folder** shows it; **Delete frames** deletes its frames and flicker
thumbnails but keeps the work time, the export history and your videos; once a recording has no
frames, **Delete folder** removes it from the library. Both ask first, and both are refused while
that recording is recording or a timelapse is being exported. Deleting a folder only removes
what PhotoCraft wrote (`recording.json`, `thumbs32.bin`, `frames/`): a video you saved inside it
stays, and so does the folder then.

## Exporting

File › Timelapse › Export Timelapse… opens the panel's export box: length (Auto: every frame
once, 5 s – 1 min; or 10 s – 5 min), and under Advanced settings: show the finished drawing first,
flash before the final image, hold the final image, slow down at the end, remove flicker (the
TimeSplice method at sensitivity 200; 32×32 thumbnails cached in `thumbs32.bin`), a work-time
counter in a corner, and the bars colour for frames from a different canvas size.

**MP4 video (ffmpeg)** pipes the frames as raw RGB into ffmpeg (H.264, CRF 18, BT.709, yuv420p,
`+faststart`), so progress is exact and Cancel stops it at once. ffmpeg is looked for in this order:
Settings › ffmpeg, `PHOTOCRAFT_FFMPEG`, next to the PhotoCraft executable (or its `ffmpeg/` or
`ffmpeg/bin/` folder), then `PATH`. **JPEG frames** writes `frame_000001.jpg …` into a folder
instead and needs no ffmpeg. The export runs as a background job (progress, Cancel).

## On disk

```text
<recordings folder>/
  Portrait (1a2b3c4d)/
    recording.json         frame list, work time, export history (the plug-in's format;
                           keys PhotoCraft doesn't know are kept)
    frames/f_000001.jpg …  the captured frames (sRGB, longest side 1280 / 1920 / 2880)
    thumbs32.bin           flicker-check thumbnails
```

The settings live in the preferences under `dialogs["file.timelapse"]`.

## Commands

| Command | |
|---|---|
| `file.timelapse.record` | start or resume recording the active document |
| `file.timelapse.pause` | pause it |
| `file.timelapse.export` | `{path, encoder?, lengthSec?, introSec?, flash?, holdSec?, slowEnd?, deflicker?, timer?, corner?, letterbox?, fps?, recording?}`; a background job |
| `file.timelapse.referenceLayer` | `{reference?}` tag or untag the targeted layers |
| `file.timelapse.status` | the active document's recording, the settings |
| `file.timelapse.settings` | get / set `{folder, quality, hideUndone, referenceTag, idleSec, autoRecord, ffmpeg, export}` |
| `file.timelapse.capture` | capture a frame now |
| `file.timelapse.list` | every recording in the folder |
| `file.timelapse.deleteFrames` | `{recording}` delete a recording's frames |
| `file.timelapse.deleteRecording` | `{recording}` delete a recording that has no frames, and its folder |

They all write files or start a program, so untrusted automation (MCP, the control channel)
can't run them, nor set `dialogs["file.timelapse"]` through `prefs.set`.

Not yet: the web build (no file system).
