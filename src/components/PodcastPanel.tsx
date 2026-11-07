import { useCallback, useEffect, useMemo, useState } from "react";
import { backend } from "../engine/backend";
import { inTauri } from "../engine/bridge";
import { formatClock } from "../state/format";
import { useStore } from "../state/store";
import { tempoOf } from "../state/tempo";
import { timelineClips, type SpeakerLevel, type TrackRole } from "../state/types";
import { Icon } from "./Icon";
import { trackVar } from "./trackColor";
import { Select } from "./Select";

/** Where each speaker is levelled to, before the master. */
const LEVELS: [number, string][] = [
  [-18, "−18 LUFS · then −16 on export"],
  [-20, "−20 LUFS · quieter, more headroom"],
  [-16, "−16 LUFS · as loud as the episode"],
];

const DEPTHS: [number, string][] = [
  [-8, "8 dB down"],
  [-12, "12 dB down"],
  [-18, "18 dB down"],
];

const ROLES: [TrackRole, string][] = [
  ["speaker", "Speaker"],
  ["music", "Music"],
  ["none", "Neither"],
];

const fmt = (v: number | null | undefined) => (v == null ? "–" : v.toFixed(1).replace("-", "−"));

export function PodcastPanel() {
  const project = useStore((s) => s.project);
  // Whole seconds only: the panel need not redraw every frame while playing.
  const positionSec = useStore((s) => Math.floor(s.positionSec));
  const busy = useStore((s) => s.busy);
  const { togglePodcast, setTrackRole, levelSpeakers, duckMusic, addChapter, renameMarker, removeMarker, seekBeats, exportChapters, setExportOpen } = useStore.getState();
  const [target, setTarget] = useState(-18);
  const [depth, setDepth] = useState(-12);
  const [levels, setLevels] = useState<Record<string, SpeakerLevel>>({});

  const audio = project.tracks.filter((t) => t.kind === "audio");
  const speakers = audio.filter((t) => t.role === "speaker");
  const music = audio.filter((t) => t.role === "music");
  const tempo = tempoOf(project);
  const chapters = useMemo(() => [...project.markers].sort((a, b) => a.start - b.start), [project.markers]);
  // Music that also ducks through compressors keyed by a voice (as the template sets up).
  const keyed = music.some((t) => t.devices.some((d) => d.on && d.sidechain && speakers.some((s) => s.id === d.sidechain)));

  // What each track plays changes with its clips and their audio (leveling, undo): measure again then.
  const playing = audio
    .map((t) => `${t.id}:${timelineClips(t).map((c) => `${c.id}/${c.kind === "audio" ? `${c.asset}/${c.gain ?? 0}` : ""}`).join(",")}`)
    .join("|");
  const measure = useCallback(() => {
    if (!inTauri) return;
    const ids = useStore.getState().project.tracks.filter((t) => t.kind === "audio").map((t) => t.id);
    void backend
      .podcastMeasure(ids)
      .then((list) => setLevels(Object.fromEntries(list.map((l) => [l.track, l]))))
      .catch(() => setLevels({}));
  }, []);
  useEffect(measure, [measure, playing]);

  const spread = (() => {
    const l = speakers.map((t) => levels[t.id]?.loudness).filter((v): v is number => v != null);
    return l.length > 1 ? Math.max(...l) - Math.min(...l) : null;
  })();

  return (
    <aside className="analysis podcast card shine" aria-label="Podcast">
      <div className="ph">
        <h2 className="pt">Podcast</h2>
        <button className="ib" type="button" aria-label="Close the podcast tools" onClick={togglePodcast}>
          <Icon name="close" size={15} />
        </button>
      </div>
      <div className="an-scroll">
        <section className="an-section" aria-labelledby="pod-who">
          <div className="an-head">
            <h3 id="pod-who">Who is who</h3>
            {spread !== null && <b className={spread > 3 ? "warn" : undefined}>{spread > 3 ? `${fmt(spread)} LU apart` : "Speakers match"}</b>}
          </div>
          {audio.length === 0 ? (
            <p className="muted an-note">Add an audio track for each voice, and one for music.</p>
          ) : (
            <ul className="pod-tracks">
              {audio.map((t) => {
                const level = levels[t.id];
                return (
                  <li key={t.id}>
                    <span className="pod-name">
                      <span className="sw" style={{ background: trackVar(t.color) }} aria-hidden="true" />
                      {t.name}
                    </span>
                    <span className="pod-lufs mono" title="Loudness of what the track plays">
                      {level?.loudness != null ? `${fmt(level.loudness)} LUFS` : timelineClips(t).length ? "–" : "empty"}
                    </span>
                    <div className="seg sm" role="group" aria-label={`${t.name} is`}>
                      {ROLES.map(([role, label]) => (
                        <button key={role} type="button" aria-pressed={(t.role ?? "none") === role} onClick={() => setTrackRole(t.id, role)}>
                          {label}
                        </button>
                      ))}
                    </div>
                  </li>
                );
              })}
            </ul>
          )}
        </section>

        <section className="an-section" aria-labelledby="pod-level">
          <h3 id="pod-level">Even out the speakers</h3>
          <p className="muted an-note">
            Rides each speaker’s level towards the same loudness, by up to 12 dB, holding still through pauses so the room between words stays down.
          </p>
          <label className="an-target">
            To
            <Select value={target} onChange={(e) => setTarget(Number(e.target.value))} aria-label="Level the speakers to">
              {LEVELS.map(([v, label]) => (
                <option key={v} value={v}>
                  {label}
                </option>
              ))}
            </Select>
          </label>
          <button
            className="btn sm"
            type="button"
            disabled={!speakers.length || !!busy}
            onClick={() => void levelSpeakers(speakers.map((t) => t.id), target).then(measure)}
          >
            <Icon name="sliders" size={14} />
            {speakers.length ? `Level ${speakers.length === 1 ? speakers[0].name : `${speakers.length} speakers`}` : "Mark a speaker first"}
          </button>
        </section>

        <section className="an-section" aria-labelledby="pod-duck">
          <h3 id="pod-duck">Duck the music</h3>
          <p className="muted an-note">Writes a volume lane on the music that dips wherever someone speaks. Move its points to adjust.</p>
          <label className="an-target">
            Under speech
            <Select value={depth} onChange={(e) => setDepth(Number(e.target.value))} aria-label="How far the music dips">
              {DEPTHS.map(([v, label]) => (
                <option key={v} value={v}>
                  {label}
                </option>
              ))}
            </Select>
          </label>
          <button
            className="btn sm"
            type="button"
            disabled={!music.length || !speakers.length || !!busy}
            onClick={() => void duckMusic(music.map((t) => t.id), speakers.map((t) => t.id), depth)}
          >
            <Icon name="automation" size={14} />
            {!music.length ? "Mark a music track first" : !speakers.length ? "Mark a speaker first" : `Duck ${music.length === 1 ? music[0].name : "the music"}`}
          </button>
          {keyed && <p className="muted an-note">The music also ducks through compressors listening to the voices. Switch them off if it dips too far.</p>}
        </section>

        <section className="an-section" aria-labelledby="pod-chapters">
          <div className="an-head">
            <h3 id="pod-chapters">Chapters</h3>
            <span className="muted">{chapters.length ? `${chapters.length} from markers` : "none yet"}</span>
          </div>
          {chapters.length > 0 && (
            <ol className="pod-chapters">
              {chapters.map((m, i) => (
                <li key={m.id}>
                  <button className="pod-time mono" type="button" title="Go there" onClick={() => seekBeats(m.start)}>
                    {formatClock(tempo.secondsAt(m.start))}
                  </button>
                  <input
                    className="pod-title"
                    aria-label={`Chapter ${i + 1} title`}
                    defaultValue={m.name}
                    key={m.name}
                    onBlur={(e) => e.target.value.trim() && e.target.value !== m.name && renameMarker(m.id, e.target.value.trim())}
                    onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
                  />
                  <button className="ib sm" type="button" aria-label={`Remove chapter ${m.name}`} onClick={() => removeMarker(m.id)}>
                    <Icon name="close" size={13} />
                  </button>
                </li>
              ))}
            </ol>
          )}
          <div className="an-actions">
            <button className="btn sm" type="button" onClick={addChapter}>
              <Icon name="plus" size={13} />
              Chapter at {formatClock(positionSec)}
            </button>
            <button className="btn sm btn-ghost" type="button" disabled={!chapters.length} onClick={() => void exportChapters()}>
              Save chapters…
            </button>
          </div>
        </section>

        <section className="an-section" aria-labelledby="pod-publish">
          <h3 id="pod-publish">Publish</h3>
          <p className="muted an-note">An MP3 at −16 LUFS with a limiter on the peaks, its chapters inside, and a chapters file for your podcast host.</p>
          <button
            className="btn btn-primary sm"
            type="button"
            onClick={() => setExportOpen(true, { format: "mp3", bitrate: 128, loudness: -16, limit: true, chapters: true, chaptersFile: "json" })}
          >
            <Icon name="export" size={14} />
            Export the episode…
          </button>
        </section>
      </div>
    </aside>
  );
}
