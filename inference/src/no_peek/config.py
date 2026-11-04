"""Pemuat konfigurasi .env lintas komponen (tanpa dependensi eksternal)."""

from dataclasses import dataclass
import os
from pathlib import Path


def find_repo_root() -> Path:
    """Temukan direktori root repository No-Peek tempat .env dan model/ berada."""
    current = Path(__file__).resolve()
    for parent in current.parents:
        if (parent / "model").is_dir() or (parent / ".env").is_file() or (parent / ".env.example").is_file():
            return parent
    return current.parents[3]


def load_dotenv(dotenv_path: Path | None = None) -> dict[str, str]:
    """Baca file .env sederhana dan masukkan ke os.environ bila variabel belum diset."""
    path = dotenv_path or (find_repo_root() / ".env")
    loaded: dict[str, str] = {}
    if not path.is_file():
        return loaded
    for raw_line in path.read_text(encoding="utf-8").splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, _, value = line.partition("=")
        key = key.strip()
        value = value.strip().strip('"').strip("'")
        if key:
            loaded[key] = value
            os.environ.setdefault(key, value)
    return loaded


def _env_bool(key: str, default: bool) -> bool:
    val = os.environ.get(key)
    if val is None:
        return default
    return val.strip().lower() in ("1", "true", "yes", "on")


@dataclass(frozen=True)
class AppConfig:
    """Konfigurasi runtime Python inference yang disinkronkan dengan .env."""

    env_mode: str = "dev"
    ipc_mode: str = "stdio"
    ipc_host: str = "127.0.0.1"
    ipc_port: int = 17890
    ipc_heartbeat_s: float = 2.0
    camera_index: int = 0
    camera_width: int = 1280
    camera_height: int = 720
    mirror: bool = True
    yolo_model: Path = Path("model/yolov8n-face.onnx")
    pose_model: Path = Path("model/face_landmarker.task")
    onnx_threads: int = 2
    confidence: float = 0.50
    iou: float = 0.45
    max_yaw: float = 35.0
    max_pitch: float = 25.0
    max_roll: float = 35.0
    fov_deg: float = 60.0
    friend_margin_cm: float = 25.0
    auto_calibrate: bool = True
    text_alert: str = "Pengintip Terdeteksi"
    text_escalated: str = "Bahaya! Layar Sedang Diintip"

    @property
    def is_dev(self) -> bool:
        """True bila mode dev (tampilkan preview kamera OpenCV)."""
        return self.env_mode.lower() == "dev"

    @property
    def headless(self) -> bool:
        """Pada mode prod, jendela OpenCV kamera disembunyikan (headless=True)."""
        return not self.is_dev

    @classmethod
    def from_env(cls, dotenv_path: Path | None = None) -> "AppConfig":
        """Muat .env dari root repository dan bangun AppConfig."""
        load_dotenv(dotenv_path)
        root = find_repo_root()
        yolo_rel = os.environ.get("NOPEEK_YOLO_MODEL", "model/yolov8n-face.onnx")
        pose_rel = os.environ.get("NOPEEK_POSE_MODEL", "model/face_landmarker.task")
        yolo_path = Path(yolo_rel) if Path(yolo_rel).is_absolute() else (root / yolo_rel)
        pose_path = Path(pose_rel) if Path(pose_rel).is_absolute() else (root / pose_rel)
        return cls(
            env_mode=os.environ.get("NOPEEK_ENV", "dev").strip().lower(),
            ipc_mode=os.environ.get("NOPEEK_IPC_MODE", "stdio").strip().lower(),
            ipc_host=os.environ.get("NOPEEK_IPC_HOST", "127.0.0.1").strip(),
            ipc_port=int(os.environ.get("NOPEEK_IPC_PORT", "17890")),
            ipc_heartbeat_s=float(os.environ.get("NOPEEK_IPC_HEARTBEAT_S", "2.0")),
            camera_index=int(os.environ.get("NOPEEK_CAMERA_INDEX", "0")),
            camera_width=int(os.environ.get("NOPEEK_CAMERA_WIDTH", "1280")),
            camera_height=int(os.environ.get("NOPEEK_CAMERA_HEIGHT", "720")),
            mirror=_env_bool("NOPEEK_MIRROR", True),
            yolo_model=yolo_path,
            pose_model=pose_path,
            onnx_threads=int(os.environ.get("NOPEEK_ONNX_THREADS", "2")),
            confidence=float(os.environ.get("NOPEEK_CONFIDENCE", "0.50")),
            iou=float(os.environ.get("NOPEEK_IOU", "0.45")),
            max_yaw=float(os.environ.get("NOPEEK_MAX_YAW", "35.0")),
            max_pitch=float(os.environ.get("NOPEEK_MAX_PITCH", "25.0")),
            max_roll=float(os.environ.get("NOPEEK_MAX_ROLL", "35.0")),
            fov_deg=float(os.environ.get("NOPEEK_FOV_DEG", "60.0")),
            friend_margin_cm=float(os.environ.get("NOPEEK_FRIEND_MARGIN_CM", "25.0")),
            auto_calibrate=_env_bool("NOPEEK_AUTO_CALIBRATE", True),
            text_alert=os.environ.get("NOPEEK_TEXT_ALERT", "Pengintip Terdeteksi"),
            text_escalated=os.environ.get("NOPEEK_TEXT_ESCALATED", "Bahaya! Layar Sedang Diintip"),
        )
