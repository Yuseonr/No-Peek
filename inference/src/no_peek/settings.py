from dataclasses import dataclass, replace
from math import isfinite


@dataclass(frozen=True)
class Settings:
    confidence: float
    alert_after_s: float
    escalated_after_s: float
    preview: bool
    mirror: bool

    def __post_init__(self) -> None:
        if not all(isfinite(v) for v in (self.confidence, self.alert_after_s, self.escalated_after_s)):
            raise ValueError("Pengaturan harus berupa angka finite")
        if not 0 < self.confidence <= 1 or not 0 < self.alert_after_s < self.escalated_after_s:
            raise ValueError("Confidence harus (0,1], dan 0 < alert < escalated")

    def changed(self, command: dict) -> "Settings":
        action = command.get("cmd")
        if action == "set_preview":
            enabled = command.get("enabled")
            if type(enabled) is not bool:
                raise ValueError("Preview enabled harus boolean")
            return replace(self, preview=enabled)
        if action == "toggle_preview":
            return replace(self, preview=not self.preview)
        if action == "toggle_mirror":
            return replace(self, mirror=not self.mirror)
        if action == "set_confidence":
            percent = command.get("percent")
            if type(percent) is not int or percent not in (35, 50, 70):
                raise ValueError("Pilih confidence 35, 50, atau 70 persen")
            return replace(self, confidence=percent / 100)
        if action == "set_alert_delay":
            seconds = command.get("seconds")
            if type(seconds) is not int or seconds not in (2, 3, 5):
                raise ValueError("Pilih jeda 2, 3, atau 5 detik")
            return replace(self, alert_after_s=float(seconds), escalated_after_s=float(seconds + 5))
        raise ValueError(f"Perintah pengaturan tidak dikenal: {action}")

    def environment(self) -> dict[str, str]:
        return {
            "NOPEEK_CONFIDENCE": str(self.confidence),
            "NOPEEK_ALERT_AFTER_S": str(self.alert_after_s),
            "NOPEEK_ESCALATED_AFTER_S": str(self.escalated_after_s),
            "NOPEEK_PREVIEW": str(self.preview).lower(),
            "NOPEEK_MIRROR": str(self.mirror).lower(),
        }

    def payload(self) -> dict:
        return {"type": "settings", **self.__dict__}
