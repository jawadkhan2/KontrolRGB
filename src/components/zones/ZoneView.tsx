import type { ZoneInfo } from "../../types/device";
import { KeyboardLayout } from "./KeyboardLayout";
import { LedStrip } from "./LedStrip";

interface Props {
  deviceId: string;
  zone: ZoneInfo;
}

export function ZoneView({ deviceId, zone }: Props) {
  if (zone.keys) {
    return <KeyboardLayout deviceId={deviceId} zone={zone} keys={zone.keys} />;
  }
  return <LedStrip deviceId={deviceId} zone={zone} />;
}
