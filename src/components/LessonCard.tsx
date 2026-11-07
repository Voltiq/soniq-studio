import { useEffect, useRef } from "react";
import { LESSONS } from "../state/lessons";
import { useStore, type Store } from "../state/store";
import { Icon } from "./Icon";

/** The element a step points at: the first match whose text contains `text`. */
function findSpot(selector: string, text?: string): HTMLElement | null {
  const all = [...document.querySelectorAll<HTMLElement>(selector)];
  return all.find((el) => !text || el.textContent?.includes(text)) ?? null;
}

/** The lesson being followed: its step, a pointer at the control, and Back/Next. */
export function LessonCard() {
  const lesson = useStore((s) => s.lesson);
  const { setLessonStep, endLesson } = useStore.getState();
  const def = LESSONS.find((l) => l.id === lesson?.id);
  const step = def && lesson ? def.steps[lesson.step] : undefined;
  // The app as it was when the step began, for steps that need "more than before".
  const start = useRef<Store | null>(null);

  // Moves on by itself once the step is done.
  useEffect(() => {
    if (!def || !lesson || !step) return;
    start.current = useStore.getState();
    // Moving on changes the store, which calls this again before it is
    // replaced: once is enough.
    let moved = false;
    const check = (s: Store) => {
      if (moved || !step.done || !start.current || !step.done(s, start.current)) return;
      moved = true;
      if (lesson.step + 1 < def.steps.length) setLessonStep(lesson.step + 1);
      else endLesson(true);
    };
    check(useStore.getState());
    return useStore.subscribe(check);
  }, [def, lesson, step, setLessonStep, endLesson]);

  // Points at the step's control, also when it appears later.
  useEffect(() => {
    if (!step?.spot) return;
    const { selector, text } = step.spot;
    let marked: HTMLElement | null = null;
    const mark = () => {
      const el = findSpot(selector, text);
      if (el === marked) return;
      marked?.classList.remove("lesson-spot");
      el?.classList.add("lesson-spot");
      marked = el;
    };
    mark();
    const id = window.setInterval(mark, 400);
    return () => {
      window.clearInterval(id);
      marked?.classList.remove("lesson-spot");
    };
  }, [step]);

  if (!def || !lesson || !step) return null;
  const last = lesson.step === def.steps.length - 1;
  return (
    <aside className="lesson glass" role="dialog" aria-label={`Lesson: ${def.title}`}>
      <header className="lesson-head">
        <Icon name="book" size={15} className="dim" />
        <span className="lesson-name">{def.title}</span>
        <span className="muted">
          {lesson.step + 1} of {def.steps.length}
        </span>
        <button className="ib xs" type="button" aria-label="Leave the lesson" onClick={() => endLesson(false)}>
          <Icon name="close" size={14} />
        </button>
      </header>
      <div className="lesson-progress" aria-hidden="true">
        {def.steps.map((_, i) => (
          <span key={i} className={i < lesson.step ? "done" : i === lesson.step ? "on" : undefined} />
        ))}
      </div>
      <h3 className="lesson-title">{step.title}</h3>
      <p className="lesson-text">{step.text}</p>
      <footer className="lesson-foot">
        {step.action && (
          <button className="btn sm btn-primary" type="button" onClick={() => void step.action?.run(useStore.getState())}>
            {step.action.label}
          </button>
        )}
        <span className="lesson-gap" />
        <button className="btn sm btn-ghost" type="button" disabled={lesson.step === 0} onClick={() => setLessonStep(lesson.step - 1)}>
          Back
        </button>
        <button className="btn sm" type="button" onClick={() => (last ? endLesson(true) : setLessonStep(lesson.step + 1))}>
          {last ? "Finish" : "Skip"}
        </button>
      </footer>
    </aside>
  );
}
