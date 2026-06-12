import { useEffect } from "react";
import { DevicePage } from "./components/DevicePage";
import { Sidebar } from "./components/Sidebar";
import { startEventListeners } from "./lib/events";
import { useDevices } from "./store/devices";

export default function App() {
  const init = useDevices((s) => s.init);

  useEffect(() => {
    const stop = startEventListeners();
    void init();
    return stop;
  }, [init]);

  return (
    <div className="flex h-full">
      <Sidebar />
      <DevicePage />
    </div>
  );
}
