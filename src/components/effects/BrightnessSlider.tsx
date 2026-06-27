import { useEffect, useRef, useState } from "react";

interface Props {
  value: number;
  onChange: (value: number) => void;
}

export function BrightnessSlider({ value, onChange }: Props) {
  const [draft, setDraft] = useState(value);
  const timer = useRef<number | null>(null);

  useEffect(() => {
    setDraft(value);
  }, [value]);

  useEffect(
    () => () => {
      if (timer.current !== null) window.clearTimeout(timer.current);
    },
    [],
  );

  const commit = (next: number) => {
    if (timer.current !== null) window.clearTimeout(timer.current);
    timer.current = null;
    onChange(next);
  };

  const queue = (next: number) => {
    setDraft(next);
    if (timer.current !== null) window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => commit(next), 120);
  };

  return (
    <div className="flex items-center gap-2">
      <span className="text-sm text-zinc-400">☀️</span>
      <input
        type="range"
        min={0}
        max={100}
        value={draft}
        onChange={(e) => queue(Number(e.target.value))}
        onPointerUp={() => commit(draft)}
        onKeyUp={(e) => {
          if (e.key.startsWith("Arrow")) commit(draft);
        }}
        className="w-36 accent-(--color-accent)"
        title={`Brightness ${draft}%`}
      />
      <span className="w-9 text-right text-xs tabular-nums text-zinc-400">
        {draft}%
      </span>
    </div>
  );
}
