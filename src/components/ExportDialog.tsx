import { useEffect, useState } from "react";
import { formatClock } from "../state/format";
import { useStore, type ExportMode } from "../state/store";
import { tempoOf } from "../state/tempo";
import { clipLength, timelineClips, type ChaptersFile, type Encoding, type ExportFormat, type Metadata } from "../state/types";
import { Icon } from "./Icon";

const TARGETS: [string, string][] = [
  ["", "Off · keep the mix level"],
  ["-14", "−14 LUFS · Spotify, YouTube, Apple Music"],
  ["-16", "−16 LUFS · podcasts"],
  ["-23", "−23 LUFS · broadcast (EBU R128)"],
];

const FORMATS: [ExportFormat, string][] = [
  ["wav", "WAV"],
  ["flac", "FLAC"],
  ["mp3", "MP3"],
  ["ogg", "OGG"],
  ["opus", "Opus"],
];

const FORMAT_NOTES: Record<ExportFormat, string> = {
  wav: "Uncompressed: the safest choice for further editing.",
  flac: "Lossless and about half the size of WAV.",
  mp3: "Plays everywhere. 192 kbps or more sounds transparent to most ears.",
  ogg: "Ogg Vorbis: smaller than MP3 at the same quality.",
  opus: "The most efficient codec: great at low bitrates, always 48 kHz.",
};

const EMPTY_TAGS: Metadata = { title: "", artist: "", album: "", year: "", genre: "", track: "", comment: "" };

/** The encoding choices remembered for this session, shared by export and batch processing. */
let lastEncoding: Encoding = { format: "wav", bits: 24, bitrate: 192, quality: 6, metadata: EMPTY_TAGS };

export function defaultEncoding(): Encoding {
  return { ...lastEncoding, metadata: { ...EMPTY_TAGS } };
}

/** Format plus its bit depth, bitrate or quality. */
export function EncodingFields({ encoding, onChange }: { encoding: Encoding; onChange: (next: Encoding) => void }) {
  const set = (patch: Partial<Encoding>) => {
    const next = { ...encoding, ...patch };
    lastEncoding = { ...next, metadata: EMPTY_TAGS };
    onChange(next);
  };
  const { format } = encoding;
  const lossless = format === "wav" || format === "flac";
  return (
    <>
      <div className="setting wide">
        <span className="lab">Format</span>
        <div className="seg" role="group" aria-label="Format">
          {FORMATS.map(([f, label]) => (
            <button key={f} type="button" aria-pressed={format === f} onClick={() => set({ format: f, bits: f === "flac" && encoding.bits === 32 ? 24 : encoding.bits })}>
              {label}
            </button>
          ))}
        </div>
      </div>
      {lossless && (
        <div className="setting">
          <span className="lab">Bit depth</span>
          <div className="seg" role="group" aria-label="Bit depth">
            {([16, 24, 32] as const).map((b) => (
              <button key={b} type="button" aria-pressed={encoding.bits === b} disabled={format === "flac" && b === 32} onClick={() => set({ bits: b })}>
                {b === 32 ? "32-bit float" : `${b}-bit`}
              </button>
            ))}
          </div>
        </div>
      )}
      {(format === "mp3" || format === "opus") && (
        <label className="setting">
          <span className="lab">Bitrate</span>
          <span className="selwrap">
            <select className="select" value={encoding.bitrate} onChange={(e) => set({ bitrate: Number(e.target.value) })}>
              {(format === "mp3" ? [128, 160, 192, 256, 320] : [64, 96, 128, 160, 192, 256]).map((k) => (
                <option key={k} value={k}>
                  {k} kbps{format === "mp3" && k === 320 ? " · best" : format === "opus" && k === 128 ? " · transparent for most music" : ""}
                </option>
              ))}
            </select>
            <Icon name="chevronDown" size={14} />
          </span>
        </label>
      )}
      {format === "ogg" && (
        <label className="setting">
          <span className="lab">Quality</span>
          <span className="selwrap">
            <select className="select" value={encoding.quality} onChange={(e) => set({ quality: Number(e.target.value) })}>
              {[2, 4, 5, 6, 8, 10].map((q) => (
                <option key={q} value={q}>
                  {q} · about {[0, 0, 96, 0, 128, 160, 192, 0, 256, 0, 500][q]} kbps
                </option>
              ))}
            </select>
            <Icon name="chevronDown" size={14} />
          </span>
        </label>
      )}
    </>
  );
}

function TagFields({ tags, onChange }: { tags: Metadata; onChange: (next: Metadata) => void }) {
  const field = (key: keyof Metadata, label: string, wide = false) => (
    <label className={wide ? "setting wide" : "setting"}>
      <span className="lab">{label}</span>
      <span className="field">
        <input value={tags[key]} onChange={(e) => onChange({ ...tags, [key]: e.target.value })} />
      </span>
    </label>
  );
  return (
    <div className="settings-grid tags">
      {field("title", "Title")}
      {field("artist", "Artist")}
      {field("album", "Album")}
      {field("genre", "Genre")}
      {field("year", "Year")}
      {field("track", "Track number")}
      {field("comment", "Comment", true)}
    </div>
  );
}

function Export({ onClose }: { onClose: () => void }) {
  const project = useStore((s) => s.project);
  const loop = useStore((s) => s.loop);
  const engineRate = useStore((s) => s.engineStatus?.sampleRate ?? 48_000);
  const exportMix = useStore((s) => s.exportMix);
  // Podcast mode opens the dialog with an episode's choices.
  const [preset] = useState(() => useStore.getState().exportPreset);
  const [encoding, setEncoding] = useState<Encoding>(() => ({
    ...defaultEncoding(),
    ...(preset?.format ? { format: preset.format } : {}),
    ...(preset?.bitrate ? { bitrate: preset.bitrate } : {}),
    metadata: { ...EMPTY_TAGS, title: project.name },
  }));
  const [rate, setRate] = useState<string>("");
  const [mode, setMode] = useState<ExportMode>("song");
  const [loudness, setLoudness] = useState(preset?.loudness != null ? String(preset.loudness) : "");
  const [limit, setLimit] = useState(preset?.limit ?? false);
  const [chapters, setChapters] = useState(preset?.chapters ?? false);
  const [chaptersFile, setChaptersFile] = useState<ChaptersFile | "">(preset?.chaptersFile ?? "");
  const [showTags, setShowTags] = useState(false);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  // The song ends where the last clip ends (in seconds, the tempo track included).
  const tempo = tempoOf(project);
  const songEnd = Math.max(0, ...project.tracks.flatMap((t) => timelineClips(t).map((c) => tempo.secondsAt(c.start + clipLength(c, tempo)))));
  const [start, end] = mode === "loop" ? [tempo.secondsAt(loop.start), tempo.secondsAt(loop.end)] : [0, songEnd];
  const regions = project.markers.filter((m) => m.end !== undefined).length;
  const markers = project.markers.length;
  // Chapters belong to one file: the song or the loop range.
  const oneFile = mode === "song" || mode === "loop";
  const withChapters = oneFile && chapters && markers > 0;
  const stems = project.tracks.filter((t) => timelineClips(t).length > 0).length;
  const { format, bits } = encoding;
  const count = mode === "regions" ? regions : mode === "stems" ? stems : 1;

  const go = () =>
    void exportMix(
      {
        ...encoding,
        bits: format === "flac" && bits === 32 ? 24 : bits,
        sampleRate: rate && format !== "opus" ? Number(rate) : null,
        start,
        end,
        loudness: loudness ? Number(loudness) : null,
        limit: !!loudness && limit,
        chapters: withChapters,
        chaptersFile: withChapters && chaptersFile ? chaptersFile : null,
      },
      mode,
    );

  const rangeNote =
    mode === "regions"
      ? `${regions} ${regions === 1 ? "region" : "regions"}, one file each, named after the region.`
      : mode === "stems"
        ? `${stems} ${stems === 1 ? "track" : "tracks"}, one file each, every one alone at its own volume and pan.`
        : `${formatClock(start)} – ${formatClock(end)} · ${formatClock(Math.max(0, end - start))} long.`;

  return (
    <div className="scrim" onPointerDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="dialog shine export-dialog" role="dialog" aria-modal="true" aria-labelledby="export-title">
        <header className="dialog-head">
          <h2 id="export-title">Export</h2>
          <button className="ib" type="button" aria-label="Close" onClick={onClose} autoFocus>
            <Icon name="close" size={16} />
          </button>
        </header>

        <div className="settings-grid">
          <EncodingFields encoding={encoding} onChange={setEncoding} />
          {format !== "opus" && (
            <label className="setting">
              <span className="lab">Sample rate</span>
              <span className="selwrap">
                <select className="select" value={rate} onChange={(e) => setRate(e.target.value)}>
                  <option value="">Engine rate ({engineRate / 1000} kHz)</option>
                  {[44_100, 48_000, 88_200, 96_000]
                    .filter((r) => r !== engineRate && (format !== "mp3" || r <= 48_000))
                    .map((r) => (
                      <option key={r} value={r}>
                        {r / 1000} kHz
                      </option>
                    ))}
                </select>
                <Icon name="chevronDown" size={14} />
              </span>
            </label>
          )}
          <div className="setting wide">
            <span className="lab">What</span>
            <div className="seg" role="group" aria-label="What to export">
              <button type="button" aria-pressed={mode === "song"} onClick={() => setMode("song")}>
                Whole song
              </button>
              <button type="button" aria-pressed={mode === "loop"} onClick={() => setMode("loop")}>
                Loop region
              </button>
              <button type="button" aria-pressed={mode === "regions"} disabled={!regions} onClick={() => setMode("regions")}>
                Each region ({regions})
              </button>
              <button type="button" aria-pressed={mode === "stems"} disabled={!stems} onClick={() => setMode("stems")}>
                Stems ({stems})
              </button>
            </div>
          </div>
          <label className="setting wide">
            <span className="lab">Loudness</span>
            <span className="selwrap">
              <select className="select" value={loudness} onChange={(e) => setLoudness(e.target.value)}>
                {TARGETS.map(([v, label]) => (
                  <option key={v} value={v}>
                    {label}
                  </option>
                ))}
              </select>
              <Icon name="chevronDown" size={14} />
            </span>
          </label>
          {loudness && (
            <div className="setting wide switch-row">
              <button className="switch sm" type="button" role="switch" aria-checked={limit} aria-labelledby="limit-label" onClick={() => setLimit(!limit)} />
              <span id="limit-label">Reach the target with a limiter, holding peaks at −1 dBFS</span>
            </div>
          )}
          {oneFile && markers > 0 && (
            <div className="setting wide switch-row">
              <button className="switch sm" type="button" role="switch" aria-checked={chapters} aria-labelledby="chapters-label" onClick={() => setChapters(!chapters)} />
              <span id="chapters-label">
                Chapters from the {markers} {markers === 1 ? "marker" : "markers"}
                {format === "mp3" ? " (ID3 chapters)" : format === "wav" ? " (cue points)" : " (chapter comments)"}
              </span>
              {chapters && (
                <span className="selwrap sm">
                  <select className="select" aria-label="Chapters file" value={chaptersFile} onChange={(e) => setChaptersFile(e.target.value as ChaptersFile | "")}>
                    <option value="">No chapters file</option>
                    <option value="json">+ chapters.json · podcast hosts</option>
                    <option value="txt">+ chapters.txt · show notes</option>
                    <option value="cue">+ .cue sheet</option>
                  </select>
                  <Icon name="chevronDown" size={14} />
                </span>
              )}
            </div>
          )}
        </div>

        <button className="disclosure" type="button" aria-expanded={showTags} onClick={() => setShowTags(!showTags)}>
          <Icon name="chevronRight" size={14} />
          Tags
          <span className="muted">{encoding.metadata.title || encoding.metadata.artist ? `${encoding.metadata.title}${encoding.metadata.artist ? ` · ${encoding.metadata.artist}` : ""}` : "none"}</span>
        </button>
        {showTags && <TagFields tags={encoding.metadata} onChange={(metadata) => setEncoding({ ...encoding, metadata })} />}

        <div className="status-line">
          <Icon name="info" size={15} />
          <span>
            {rangeNote} {FORMAT_NOTES[format]}
            {(format === "wav" || format === "flac") && (format === "wav" && bits === 32 ? " No dither needed at 32-bit float." : " Dithered.")}
            {loudness && (limit ? " A limiter holds the peaks at −1 dBFS so the target is reached." : " Peaks are kept under −1 dBFS, even if that means a little under the target.")}
          </span>
        </div>

        <footer className="dialog-foot">
          <span className="muted">{count > 1 ? "You pick a folder; existing files with the same names are replaced." : "Tags are written into the file."}</span>
          <div className="tgroup">
            <button className="btn btn-ghost" type="button" onClick={onClose}>
              Cancel
            </button>
            <button className="btn btn-primary" type="button" disabled={(mode === "song" || mode === "loop") && end - start <= 0} onClick={go}>
              <Icon name="export" size={15} />
              {count > 1 ? `Export ${count} files…` : "Export…"}
            </button>
          </div>
        </footer>
      </div>
    </div>
  );
}

export function ExportDialog() {
  const open = useStore((s) => s.exportOpen);
  const setOpen = useStore((s) => s.setExportOpen);
  return open ? <Export onClose={() => setOpen(false)} /> : null;
}
