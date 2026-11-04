"""Protokol IPC JSON Lines (NDJSON) dua arah antara Python Inference dan Rust GUI."""

from collections import deque
import json
import sys
import threading
import time
from typing import TextIO

POSITION_LABELS_ID = {
    "LEFT": "Kiri",
    "CENTER": "Tengah",
    "RIGHT": "Kanan",
}


def format_alert_message(
    level: str,
    observers: list[dict],
    max_risk_s: float,
    base_alert_text: str = "Pengintip Terdeteksi",
    base_escalated_text: str = "Bahaya! Layar Sedang Diintip",
) -> str:
    """Bangun teks notifikasi kontekstual untuk Dynamic Island (tanpa emoji unicode)."""
    if level not in ("ALERT", "ESCALATED"):
        return ""
    count = len(observers)
    zones = []
    for obs in observers:
        zone_id = POSITION_LABELS_ID.get(obs.get("position", "CENTER"), "Belakang")
        if zone_id not in zones:
            zones.append(zone_id)
    zone_str = "/".join(zones) if zones else "Belakang"

    if level == "ESCALATED":
        secs = max(7, int(max_risk_s))
        if count > 1:
            return f"{base_escalated_text} ({count} Orang - {zone_str}, {secs}d+)"
        return f"{base_escalated_text} ({zone_str}, {secs}d+)"

    if count > 1:
        return f"{count} {base_alert_text} ({zone_str})"
    return f"{base_alert_text} ({zone_str})"


def build_state_payload(
    observations: list[dict],
    calibrated: bool,
    calibration_mode: str | None,
    auto_calibrate: bool,
    owner_id: int | None,
    paused: bool,
    fps: float,
    mirror: bool = True,
    base_alert_text: str = "Pengintip Terdeteksi",
    base_escalated_text: str = "Bahaya! Layar Sedang Diintip",
) -> dict:
    """Konversi hasil Analyzer.update per frame menjadi payload ringkas untuk Rust GUI."""
    active_observers = []
    highest_level = "MONITORING"
    level_rank = {"MONITORING": 0, "PENDING": 1, "ALERT": 2, "ESCALATED": 3}
    max_risk_s = 0.0

    for item in observations:
        state = item.get("state", "MONITORING")
        if level_rank.get(state, 0) > level_rank.get(highest_level, 0):
            highest_level = state
        if item.get("role") == "OBSERVER" and state in ("PENDING", "ALERT", "ESCALATED"):
            pos = item.get("position", "CENTER")
            if mirror:
                pos = {"LEFT": "RIGHT", "RIGHT": "LEFT", "CENTER": "CENTER"}.get(pos, pos)
            risk_s = round(float(item.get("risk_s", 0.0)), 2)
            max_risk_s = max(max_risk_s, risk_s)
            active_observers.append({
                "id": int(item["id"]),
                "state": state,
                "position": pos,
                "distance_cm": round(float(item.get("distance_cm", 0.0)), 1),
                "risk_s": risk_s,
            })

    alert_active = highest_level in ("ALERT", "ESCALATED")
    icon_name = "shield_alert" if highest_level == "ESCALATED" else ("eye_warning" if highest_level == "ALERT" else "none")
    message = format_alert_message(
        highest_level,
        [o for o in active_observers if o["state"] in ("ALERT", "ESCALATED")],
        max_risk_s,
        base_alert_text=base_alert_text,
        base_escalated_text=base_escalated_text,
    )

    return {
        "type": "state_update",
        "ts": round(time.time(), 3),
        "level": highest_level,
        "alert_active": alert_active,
        "icon": icon_name,
        "message": message,
        "observer_count": len([o for o in active_observers if o["state"] in ("ALERT", "ESCALATED")]),
        "max_risk_s": max_risk_s,
        "observers": active_observers,
        "calibrated": bool(calibrated),
        "calibration_mode": calibration_mode,
        "auto_calibrate": bool(auto_calibrate),
        "owner_id": owner_id,
        "paused": bool(paused),
        "fps": round(float(fps), 1),
    }


class IpcChannel:
    """Kanal IPC JSON Lines dua arah (stdout untuk event keluar, stdin untuk command masuk)."""

    def __init__(self, out_stream: TextIO | None = None, in_stream: TextIO | None = None):
        self._out = out_stream or sys.stdout
        self._in = in_stream or sys.stdin
        self._commands: deque[dict] = deque()
        self._lock = threading.Lock()
        self._last_signature: tuple | None = None
        self._last_heartbeat: float = 0.0
        self._reader_thread: threading.Thread | None = None
        self._stopped = False

    def start_reader(self) -> None:
        """Jalankan daemon thread pembaca perintah JSONL dari Rust (stdin)."""
        if self._reader_thread is not None:
            return
        self._reader_thread = threading.Thread(target=self._read_loop, name="nopeek-ipc-in", daemon=True)
        self._reader_thread.start()

    def _read_loop(self) -> None:
        try:
            for raw in self._in:
                if self._stopped:
                    break
                line = raw.strip()
                if not line:
                    continue
                try:
                    cmd = json.loads(line)
                    if isinstance(cmd, dict) and "cmd" in cmd:
                        with self._lock:
                            self._commands.append(cmd)
                except json.JSONDecodeError:
                    continue
        except (OSError, ValueError):
            pass

    def poll_command(self) -> dict | None:
        """Ambil satu perintah dari antrean masuk (non-blocking)."""
        with self._lock:
            return self._commands.popleft() if self._commands else None

    def send(self, payload: dict) -> None:
        """Kirim satu baris JSON ke Rust dan langsung flush."""
        line = json.dumps(payload, allow_nan=False, separators=(",", ":"))
        self._out.write(line + "\n")
        self._out.flush()

    def emit_state_if_needed(self, payload: dict, heartbeat_s: float = 2.0) -> bool:
        """Kirim state_update hanya saat status/teks berubah atau saat interval heartbeat tiba."""
        now = time.monotonic()
        signature = (
            payload.get("level"),
            payload.get("alert_active"),
            payload.get("message"),
            payload.get("calibrated"),
            payload.get("calibration_mode"),
            payload.get("auto_calibrate"),
            payload.get("paused"),
            payload.get("observer_count"),
        )
        if signature != self._last_signature or (now - self._last_heartbeat) >= heartbeat_s:
            self._last_signature = signature
            self._last_heartbeat = now
            self.send(payload)
            return True
        return False

    def close(self) -> None:
        """Hentikan pembaca IPC."""
        self._stopped = True
