import { useStore } from "../state/store";
import { Icon } from "./Icon";

/** The song's audio that could not be found, and two ways to find it. */
function MissingFiles({ onClose }: { onClose: () => void }) {
  const assets = useStore((s) => s.project.assets);
  const { findMissing, locateAsset } = useStore.getState();
  const missing = assets.filter((a) => a.missing);
  return (
    <div className="scrim" onPointerDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="dialog shine" role="dialog" aria-modal="true" aria-labelledby="missing-title">
        <header className="dialog-head">
          <h2 id="missing-title">Find missing files</h2>
          <button className="ib" type="button" aria-label="Close" onClick={onClose} autoFocus>
            <Icon name="close" size={16} />
          </button>
        </header>
        {missing.length === 0 ? (
          <div className="empty">
            <Icon name="check" size={22} />
            <p>
              <b>Nothing is missing</b>
              Every clip has its audio.
            </p>
          </div>
        ) : (
          <>
            <p className="muted setting-note">
              The song plays these clips silent until their audio is found. Search a folder (and the folders in it) for files with the same
              names, or point at each file yourself. Saving then copies them into the song's Samples folder.
            </p>
            <ul className="missing-list">
              {missing.map((a) => (
                <li key={a.id}>
                  <Icon name="warning" size={15} className="warn-ic" />
                  <span className="missing-what">
                    <b>{a.name}</b>
                    <span className="muted">{a.path ?? "Never saved to a file"}</span>
                  </span>
                  <button className="btn sm" type="button" onClick={() => void locateAsset(a.id)}>
                    Locate…
                  </button>
                </li>
              ))}
            </ul>
          </>
        )}
        <footer className="dialog-foot">
          <span className="muted">
            {missing.length} missing {missing.length === 1 ? "file" : "files"}
          </span>
          <div className="tgroup">
            <button className="btn btn-ghost" type="button" onClick={onClose}>
              Close
            </button>
            {missing.length > 0 && (
              <button className="btn btn-primary" type="button" onClick={() => void findMissing()}>
                <Icon name="search" size={14} />
                Search a folder…
              </button>
            )}
          </div>
        </footer>
      </div>
    </div>
  );
}

export function MissingFilesDialog() {
  const open = useStore((s) => s.missingOpen);
  const setOpen = useStore((s) => s.setMissingOpen);
  return open ? <MissingFiles onClose={() => setOpen(false)} /> : null;
}
