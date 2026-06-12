interface Props {
  value: number;
  onChange: (value: number) => void;
}

export function BrightnessSlider({ value, onChange }: Props) {
  return (
    <div className="flex items-center gap-2">
      <span className="text-sm text-zinc-400">☀️</span>
      <input
        type="range"
        min={0}
        max={100}
        value={value}
        onChange={(e) => onChange(Number(e.target.value))}
        className="w-36 accent-(--color-accent)"
        title={`Brightness ${value}%`}
      />
      <span className="w-9 text-right text-xs tabular-nums text-zinc-400">
        {value}%
      </span>
    </div>
  );
}
