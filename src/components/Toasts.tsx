import { useStore, type ToastTone } from "../state/store";
import { Icon, type IconName } from "./Icon";

const ICON: Record<ToastTone, IconName> = { success: "check", info: "info", warning: "warning", danger: "warning" };

export function Toasts() {
  const toasts = useStore((s) => s.toasts);
  const busy = useStore((s) => s.busy);
  const dismiss = useStore((s) => s.dismissToast);
  return (
    <div className="toasts" role="status" aria-live="polite">
      {busy && (
        <div className="toast glass info">
          <span className="toast-icon">
            <span className="spin" aria-hidden="true" />
          </span>
          <span className="toast-text">
            <b>{busy}</b>
          </span>
        </div>
      )}
      {toasts.map((t) => (
        <div key={t.id} className={`toast glass ${t.tone}`}>
          <span className="toast-icon">
            <Icon name={ICON[t.tone]} size={15} />
          </span>
          <span className="toast-text">
            <b>{t.title}</b>
            {t.detail && <span>{t.detail}</span>}
            {t.actions && (
              <span className="toast-actions">
                {t.actions.map((a, i) => (
                  <button key={a.label} className={i === 0 ? "btn sm btn-primary" : "btn sm"} type="button" onClick={a.run}>
                    {a.label}
                  </button>
                ))}
              </span>
            )}
          </span>
          {!t.sticky && (
            <button className="ib xs" type="button" aria-label="Dismiss" onClick={() => dismiss(t.id)}>
              <Icon name="close" size={14} />
            </button>
          )}
        </div>
      ))}
    </div>
  );
}
