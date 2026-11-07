import { useEffect, useMemo, useState } from "react";
import { inTauri } from "../engine/bridge";
import { deviceSpec } from "../state/devices";
import { useStore } from "../state/store";
import type { KeyGuess, LibraryEntry } from "../state/types";
import { startLibraryDrag } from "./dropTarget";
import { Icon } from "./Icon";
import { Popover } from "./Menu";

type Filter = "all" | "favourites" | "recent" | "loops" | "oneShots" | "presets";

const FILTERS: [Filter, string][] = [
  ["all", "All"],
  ["favourites", "Favourites"],
  ["recent", "Recent"],
  ["loops", "Loops"],
  ["oneShots", "One-shots"],
  ["presets", "Presets"],
];

/** The most items drawn at once; a search narrows the rest. */
const SHOWN = 300;
const NOTES = ["C", "C#", "D", "Eb", "E", "F", "F#", "G", "Ab", "A", "Bb", "B"];

export function keyLabel(k: KeyGuess): string {
  return k.root ? NOTES[k.tonic % 12] : `${NOTES[k.tonic % 12]} ${k.minor ? "minor" : "major"}`;
}

/** "Loop · 120 BPM · A minor · 4.0 s", or a preset's device. */
function describe(e: LibraryEntry): string {
  if (e.kind === "preset") return `${deviceSpec(e.device ?? "")?.name ?? "Device"} preset`;
  if (e.error) return e.error;
  const i = e.info;
  if (!i) return "Not listened to yet";
  const parts = [i.kind === "loop" ? "Loop" : "One-shot"];
  if (i.bpm) parts.push(`${Math.round(i.bpm * 10) / 10} BPM`);
  if (i.key) parts.push(keyLabel(i.key));
  parts.push(i.seconds < 10 ? `${i.seconds.toFixed(1)} s` : `${Math.round(i.seconds)} s`);
  return parts.join(" · ");
}

/** The user's own tags for an item, typed as words separated by commas. */
function TagEditor({ entry, anchor, onClose }: { entry: LibraryEntry; anchor: HTMLElement; onClose: () => void }) {
  const mine = useStore((s) => s.library?.tags[entry.path]);
  const [text, setText] = useState((mine ?? []).join(", "));
  const save = () => {
    void useStore.getState().setLibraryTags(entry.path, text.split(","));
    onClose();
  };
  return (
    <Popover anchor={anchor} onClose={onClose} label={`Tags for ${entry.name}`} align="right">
      <form
        className="tag-form"
        onSubmit={(e) => {
          e.preventDefault();
          save();
        }}
      >
        <label className="knob-l" htmlFor="lib-tags">
          Your tags, separated by commas
        </label>
        <input id="lib-tags" className="input sm" value={text} autoFocus onChange={(e) => setText(e.target.value)} placeholder="dark, intro, favourite kick" />
        {entry.autoTags.length > 0 && <span className="muted">From the name: {entry.autoTags.join(", ")}</span>}
        <button className="btn sm" type="submit">
          Save tags
        </button>
      </form>
    </Popover>
  );
}

function Item({ entry, owner }: { entry: LibraryEntry; owner: string | undefined }) {
  const favourite = useStore((s) => s.library?.favourites.includes(entry.path) ?? false);
  const mine = useStore((s) => s.library?.tags[entry.path]);
  const playing = useStore((s) => s.auditioning === entry.path);
  const { audition, toggleFavourite, addLibraryItem } = useStore.getState();
  const [tagAnchor, setTagAnchor] = useState<HTMLElement | null>(null);
  const preset = entry.kind === "preset";
  const tags = [...(mine ?? []), ...entry.autoTags.filter((t) => !mine?.includes(t))].slice(0, 4);
  return (
    <div role="listitem" className={`litem${playing ? " on" : ""}${entry.error && !preset ? " bad" : ""}`}>
      {preset ? (
        <span className="bicon audio-fx">
          <Icon name="sliders" size={15} />
        </span>
      ) : (
        <button
          className="ib sm lplay"
          type="button"
          aria-label={playing ? `Stop ${entry.name}` : `Hear ${entry.name}`}
          aria-pressed={playing}
          disabled={!!entry.error}
          onClick={() => void audition(entry.path)}
        >
          <Icon name={playing ? "stop" : "play"} size={13} />
        </button>
      )}
      <button
        className="ltext"
        type="button"
        title={`${entry.path}\n${preset ? "Double-click to add it to the chain on show, or drag it onto one." : "Click to hear it; double-click to add it to the song, or drag it onto a track, a pad or a sampler."}`}
        onPointerDown={(e) => startLibraryDrag(e, { path: entry.path, name: entry.name, preset })}
        onClick={() => !preset && !entry.error && void audition(entry.path)}
        onDoubleClick={() => void addLibraryItem(entry, owner)}
      >
        <span className="bname">{entry.name}</span>
        <span className="bdesc">{describe(entry)}</span>
        {tags.length > 0 && (
          <span className="ltags">
            {tags.map((t) => (
              <span key={t} className={mine?.includes(t) ? "ltag mine" : "ltag"}>
                {t}
              </span>
            ))}
          </span>
        )}
      </button>
      <button className="ib xs lmore" type="button" aria-label={`Tags for ${entry.name}`} onClick={(e) => setTagAnchor(tagAnchor ? null : e.currentTarget)}>
        <Icon name="more" size={14} />
      </button>
      <button
        className={`ib xs lstar${favourite ? " on" : ""}`}
        type="button"
        aria-label={favourite ? `Remove ${entry.name} from favourites` : `Add ${entry.name} to favourites`}
        aria-pressed={favourite}
        onClick={() => void toggleFavourite(entry.path)}
      >
        <Icon name="star" size={14} />
      </button>
      {tagAnchor && <TagEditor entry={entry} anchor={tagAnchor} onClose={() => setTagAnchor(null)} />}
    </div>
  );
}

/** The browser's library: your sample and preset folders, searchable, with favourites, tags and recent items. */
export function LibraryItems({ query, owner }: { query: string; owner: string | undefined }) {
  const library = useStore((s) => s.library);
  const sync = useStore((s) => s.auditionSync);
  const { loadLibrary, addLibraryFolder, removeLibraryFolder, rescanLibrary, setAuditionSync } = useStore.getState();
  const [filter, setFilter] = useState<Filter>("all");
  const [tag, setTag] = useState<string | null>(null);
  const [foldersOpen, setFoldersOpen] = useState(false);
  useEffect(() => {
    if (inTauri && !library) void loadLibrary();
  }, [library, loadLibrary]);

  const entries = library?.entries;
  const favourites = library?.favourites;
  const recent = library?.recent;
  const userTags = library?.tags;
  const matches = useMemo(() => {
    if (!entries) return [];
    const q = query.trim().toLowerCase();
    const words = q.split(/\s+/).filter(Boolean);
    const fav = new Set(favourites);
    let list = entries;
    if (filter === "recent") {
      const byPath = new Map(entries.map((e) => [e.path, e]));
      list = (recent ?? []).map((p) => byPath.get(p)).filter((e): e is LibraryEntry => !!e);
    } else if (filter !== "all") {
      list = entries.filter((e) =>
        filter === "favourites"
          ? fav.has(e.path)
          : filter === "presets"
            ? e.kind === "preset"
            : filter === "loops"
              ? e.info?.kind === "loop"
              : e.kind === "sample" && e.info?.kind === "oneShot",
      );
    }
    const tagsOf = (e: LibraryEntry) => [...(userTags?.[e.path] ?? []), ...e.autoTags];
    if (tag) list = list.filter((e) => tagsOf(e).includes(tag));
    if (words.length) {
      list = list.filter((e) => {
        const text = `${e.name} ${tagsOf(e).join(" ")} ${e.info?.key ? keyLabel(e.info.key) : ""} ${e.info?.bpm ? `${Math.round(e.info.bpm)}bpm` : ""} ${e.path}`.toLowerCase();
        return words.every((w) => text.includes(w));
      });
    }
    return list;
  }, [entries, favourites, recent, userTags, filter, tag, query]);

  // The tags used most, to filter by.
  const topTags = useMemo(() => {
    const count = new Map<string, number>();
    for (const e of entries ?? []) for (const t of [...(userTags?.[e.path] ?? []), ...e.autoTags]) count.set(t, (count.get(t) ?? 0) + 1);
    return [...count.entries()]
      .filter(([t]) => t !== "loop" && t !== "one-shot" && t !== "preset")
      .sort((a, b) => b[1] - a[1])
      .slice(0, 10)
      .map(([t]) => t);
  }, [entries, userTags]);

  if (!inTauri) {
    return (
      <div className="empty">
        <Icon name="folder" size={22} />
        <p>
          <b>Your library lives in the desktop app</b>
          It indexes your sample and preset folders there. Run <code>npm run tauri dev</code>.
        </p>
      </div>
    );
  }
  if (!library) return <div className="empty">Loading the library…</div>;

  const scanning = library.scanning;
  return (
    <div className="library">
      <div className="lib-bar">
        <button className="btn xs" type="button" onClick={() => void addLibraryFolder()}>
          <Icon name="plus" size={13} />
          Add folder…
        </button>
        <button className="btn xs" type="button" aria-expanded={foldersOpen} onClick={() => setFoldersOpen(!foldersOpen)}>
          {library.folders.length} {library.folders.length === 1 ? "folder" : "folders"}
          <Icon name={foldersOpen ? "chevronUp" : "chevronDown"} size={12} />
        </button>
        <label className="lib-sync" title="Loops play at the song's tempo, from the next bar while it plays; pitched sounds move to its key">
          <button className="switch sm" type="button" role="switch" aria-checked={sync} aria-label="Hear samples in time with the song" onClick={() => setAuditionSync(!sync)} />
          In time
        </label>
      </div>
      {scanning && (
        <div className="lib-scan" role="status">
          <span className="lib-scan-bar" style={{ width: scanning[1] ? `${(scanning[0] / scanning[1]) * 100}%` : "8%" }} />
          <span>{scanning[1] ? `Listening to new files: ${scanning[0]} of ${scanning[1]}` : "Looking through the folders…"}</span>
        </div>
      )}
      {foldersOpen && (
        <ul className="lib-folders">
          {library.folders.map((f) => (
            <li key={f}>
              <Icon name="folder" size={13} className="dim" />
              <span title={f}>{f}</span>
              <button className="ib xs" type="button" aria-label={`Stop indexing ${f}`} onClick={() => void removeLibraryFolder(f)}>
                <Icon name="close" size={12} />
              </button>
            </li>
          ))}
          <li className="muted">
            <Icon name="sliders" size={13} className="dim" />
            <span title={library.presets}>Presets save to {library.presets}</span>
          </li>
          <li>
            <button className="btn xs" type="button" disabled={!!scanning} onClick={() => void rescanLibrary()}>
              Look again
            </button>
          </li>
        </ul>
      )}
      <div className="chips lib-filters" role="group" aria-label="Show">
        {FILTERS.map(([id, label]) => (
          <button key={id} className="chipb sm" type="button" aria-pressed={filter === id} onClick={() => setFilter(id)}>
            {label}
          </button>
        ))}
      </div>
      {topTags.length > 0 && (
        <div className="chips lib-filters" role="group" aria-label="Tags">
          {topTags.map((t) => (
            <button key={t} className="chipb sm tagchip" type="button" aria-pressed={tag === t} onClick={() => setTag(tag === t ? null : t)}>
              {t}
            </button>
          ))}
        </div>
      )}
      <div role="list" className="lib-list">
        {library.folders.length === 0 && library.entries.length === 0 ? (
          <div className="empty">
            <Icon name="folder" size={22} />
            <p>
              <b>Add a folder of samples</b>
              Soniq listens to each file once: loop or one-shot, its tempo and key, and tags from its name. Your presets show up here too.
            </p>
          </div>
        ) : matches.length === 0 ? (
          <div className="empty">
            <p>
              <b>{scanning ? "Still looking…" : "Nothing matches"}</b>
              {filter === "favourites" ? "Star a sample to keep it here." : filter === "recent" ? "Samples you add to the song show up here." : "Try another word, tag or filter."}
            </p>
          </div>
        ) : (
          <>
            {matches.slice(0, SHOWN).map((e) => (
              <Item key={e.path} entry={e} owner={owner} />
            ))}
            {matches.length > SHOWN && <p className="muted lib-more">Showing {SHOWN} of {matches.length}: search to narrow it down.</p>}
          </>
        )}
      </div>
    </div>
  );
}

/** Saves a device as a preset file (in the library's presets folder). */
export function SavePreset({ owner, deviceId, name, anchor, onClose }: { owner: string; deviceId: string; name: string; anchor: HTMLElement; onClose: () => void }) {
  const [text, setText] = useState(name);
  const save = () => {
    void useStore.getState().saveDevicePreset(owner, deviceId, text.trim() || name);
    onClose();
  };
  return (
    <Popover anchor={anchor} onClose={onClose} label="Save as preset" align="right">
      <form
        className="tag-form"
        onSubmit={(e) => {
          e.preventDefault();
          save();
        }}
      >
        <label className="knob-l" htmlFor="preset-name">
          Preset name
        </label>
        <input id="preset-name" className="input sm" value={text} autoFocus onChange={(e) => setText(e.target.value)} />
        <span className="muted">Saved with its settings (and pad effects); samples stay with the song.</span>
        <button className="btn sm" type="submit">
          Save preset
        </button>
      </form>
    </Popover>
  );
}
