import { useEffect, useRef, useState } from "react";
import { RgbColorPicker } from "react-colorful";
import type { Color } from "../../types/device";
import { cssColor } from "../../types/device";

interface Props {
  color: Color;
  onChange: (color: Color) => void;
  /** Which edge of the swatch the popover aligns to. Use "right" when the
      swatch sits near the window's right edge so the picker isn't clipped. */
  align?: "left" | "right";
}

export function ColorPickerPopover({ color, onChange, align = "left" }: Props) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const close = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    window.addEventListener("mousedown", close);
    return () => window.removeEventListener("mousedown", close);
  }, [open]);

  return (
    <div className="relative" ref={ref}>
      <button
        onClick={() => setOpen((o) => !o)}
        className="h-8 w-14 rounded-md border border-panel-2 transition-transform hover:scale-105"
        style={{
          background: cssColor(color),
          boxShadow: `0 0 12px ${cssColor(color)}55`,
        }}
        title="Pick color"
      />
      {open && (
        <div
          className={`absolute bottom-full z-50 mb-2 rounded-xl border border-panel-2 bg-panel p-3 shadow-2xl ${
            align === "right" ? "right-0" : "left-0"
          }`}
        >
          <RgbColorPicker color={color} onChange={onChange} />
        </div>
      )}
    </div>
  );
}
