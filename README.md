# No-Peek

**Sistem Deteksi Pengintip Layar (*Shoulder Surfing*) Real-Time Menggunakan Computer Vision Lokal.**  
Didukung pada **Windows** dan **macOS** — 100% pemrosesan lokal tanpa pengiriman gambar ke *cloud*.

---

## Fitur Utama

1. **Two-Stage Computer Vision Pipeline:**
   * **Tahap 1 — Deteksi Multi-Wajah Cepat (`YOLOv8n-Face` ONNX):** Mendeteksi *bounding box* dan 5 titik *landmark* dasar wajah secara lokal di CPU menggunakan `onnxruntime` dan C++ NMS OpenCV.
   * **Tahap 2 — Estimasi Head Pose 3D (`MediaPipe Face Landmarker`):** Menghitung matriks transformasi 3D wajah untuk memperoleh sudut rotasi kepala presisi dalam derajat:
     * **Yaw** (menoleh kiri–kanan, default $|\text{yaw}| < 35^\circ$)
     * **Pitch** (mendongak–menunduk, default $|\text{pitch}| < 25^\circ$)
     * **Roll** (kemiringan kepala, default $|\text{roll}| < 35^\circ$)
2. **Estimasi Jarak Kamera ke Wajah (`distance_cm`):**
   * Menggunakan *Pinhole Camera Model* dari kombinasi lebar kotak wajah dan jarak antar-mata (*Inter-Pupillary Distance*) yang dikompensasi terhadap sudut tolehan $\cos(|\text{yaw}|)$.
3. **Auto-Kalibrasi `OWNER`:**
   * Otomatis mengunci pengguna utama (`OWNER`) yang duduk di zona tengah layar ($25\%\text{–}75\%$ lebar frame), menghadap ke layar, dan berada paling dekat dengan kamera setelah stabil selama $\ge 0{,}6\text{ detik}$.
   * Tetap mendukung kalibrasi manual kapan saja dengan tombol `C` atau *toggle* auto-kalibrasi dengan tombol `A`.
4. **Klasifikasi Role Spasial (`OWNER`, `FRIEND`, `OBSERVER`):**
   * **`OWNER` (Hijau):** Pengguna utama laptop.
   * **`FRIEND` (Biru/Cyan):** Rekan/teman yang duduk **berdampingan (*side-by-side*)** sejajar dengan `OWNER` (termasuk toleransi variasi ukuran kepala yang lebih mungil saat duduk sejajar secara vertikal). Tidak memicu *alert*.
   * **`OBSERVER` (Kuning/Merah/Ungu):** Wajah tambahan yang berada lebih jauh di belakang `OWNER` dan menghadap ke layar.
5. **Mesin Aturan Temporal (Anti-False Alarm):**
   * **`MONITORING` ($0\text{ s}$):** Kondisi aman atau pengamat sedang menoleh/menunduk.
   * **`PENDING` ($< 2\text{ s}$):** Pengamat baru menghadap layar (belum memicu peringatan).
   * **`ALERT` ($\ge 2\text{ s}$):** Pengamat menatap layar minimal 2 detik.
   * **`ESCALATED` ($\ge 7\text{ s}$):** Pengamat tetap menatap layar selama 7 detik atau lebih.

---

## Struktur Modul (`Separation of Concerns`)

```text
No-Peek/
├── .env                         # Konfigurasi mode (dev/prod), IPC, kamera, & geometri UI
├── .env.example                 # Template konfigurasi environment
├── plan.md                      # Dokumen arsitektur Dynamic Island GUI (Rust) + IPC (Python)
├── docs/
│   ├── No-Peek.md               # Dokumentasi pendekatan teknis & arsitektur CV
│   └── No-Peek.pdf
├── model/
│   ├── yolov8n-face.onnx        # Model deteksi wajah (Tahap 1)
│   └── face_landmarker.task     # Model MediaPipe 3D Head Pose (Tahap 2)
├── gui/                         # [RUST] Native Dynamic Island GUI (Windows & macOS)
│   ├── Cargo.toml
│   └── crates/
│       ├── nopeek-core/         # Geometri, pegas animasi, parser SVG, .env config, & kontrak IPC
│       └── nopeek-gui/          # Windowing & renderer native (Win32/Direct2D & AppKit/CoreGraphics)
└── inference/                   # [PYTHON] Computer Vision & Temporal Risk Engine
    ├── pyproject.toml
    ├── src/no_peek/
    │   ├── config.py            # Pemuat konfigurasi .env lintas komponen
    │   ├── ipc.py               # Kanal komunikasi IPC JSON Lines dua arah (stdout/stdin)
    │   ├── source.py            # Akuisisi frame webcam/video & manajemen clock
    │   ├── detector.py          # Inferensi YOLOv8n-Face ONNX & C++ NMS
    │   ├── pose.py              # Estimasi sudut 3D (yaw/pitch/roll) via MediaPipe
    │   ├── geometry.py          # Kalkulasi jarak (cm), zona tengah, & logika FRIEND
    │   ├── analysis.py          # Pelacakan IoU, auto-kalibrasi, & state machine risiko
    │   ├── preview.py           # Rendering UI preview & panel telemetri OpenCV (mode dev)
    │   └── main.py              # CLI entry point & orkestrasi sesi
    └── tests/
        ├── test_experiment.py   # Unit test decoder, geometri, head pose, & temporal
        ├── test_player.py       # Integrasi kontrol pemutar video & ekspor metrik
        └── test_ipc_config.py   # Unit test konfigurasi .env & protokol IPC JSON Lines
```

---

## Instalasi

Pastikan Python `>= 3.11` (direkomendasikan `3.12`) dan [`uv`](https://docs.astral.sh/uv/) telah terpasang:

```bash
uv sync
```

Atau menggunakan `venv` standar:

```powershell
python -m venv .venv
# Windows PowerShell:
.\.venv\Scripts\Activate.ps1
# macOS / Linux:
# source .venv/bin/activate

pip install -e .
```

---

## Cara Menjalankan

### 1. Menggunakan Webcam Lokal
```powershell
uv run no-peek --camera 0
# atau melalui python langsung:
.\.venv\Scripts\python.exe -m no_peek.main --camera 0
```

### 2. Evaluasi Menggunakan File Video & Ekspor Metrik JSONL
```powershell
uv run no-peek --video sample.mp4 --metrics experiments/run1.jsonl
```

### 3. Opsi Argumen CLI
| Argumen | Default | Keterangan |
| :--- | :--- | :--- |
| `--camera` | `0` | Indeks webcam lokal |
| `--video` | `None` | Path file video lokal (menggantikan webcam) |
| `--model` | `model/yolov8n-face.onnx` | Path model detektor wajah ONNX |
| `--pose-model` | `model/face_landmarker.task` | Path model MediaPipe Face Landmarker |
| `--confidence` | `0.5` | Ambang skor deteksi wajah `(0, 1]` |
| `--iou` | `0.45` | Ambang IoU untuk Non-Maximum Suppression |
| `--max-yaw` | `35.0` | Ambang mutlak sudut tolehan horizontal (`derajat`) |
| `--max-pitch` | `25.0` | Ambang mutlak sudut anggukan vertikal (`derajat`) |
| `--max-roll` | `35.0` | Ambang mutlak kemiringan kepala (`derajat`) |
| `--fov` | `60.0` | *Field of View* horizontal kamera untuk estimasi jarak |
| `--friend-margin-cm` | `25.0` | Toleransi selisih jarak belakang `OWNER` (cm) untuk `FRIEND` |
| `--auto-calibrate` / `--no-auto-calibrate` | `--auto-calibrate` | Aktifkan/nonaktifkan auto-kalibrasi `OWNER` |
| `--mirror` / `--no-mirror` | Otomatis | *Mirror* tampilan preview (default aktif pada webcam) |
| `--headless` | `False` | Jalankan tanpa jendela GUI untuk pemrosesan *batch* |
| `--metrics` | `None` | Simpan log telemetri per frame ke berkas `.jsonl` |

---

## Pintasan Keyboard (Saat Jendela Preview Aktif)

* **`A`**: *Toggle* Auto-Kalibrasi `OWNER` (`ON` / `OFF`)
* **`C`**: Kalibrasi `OWNER` secara manual (saat tepat 1 wajah terlihat)
* **`R`**: Reset pelacakan dan status kalibrasi
* **`M`**: *Toggle* tampilan *mirror* (cermin horizontal)
* **`SPACE`**: *Pause* / *Resume* (atau *Replay* saat video selesai)
* **`N` / `B`**: Maju / mundur 1 frame (khusus input file video)
* **`J` / `L`**: Mundur / maju 5 detik (khusus input file video)
* **`+` / `-`**: Percepat / perlambat kecepatan pemutaran video
* **`0`**: Ulangi video dari frame awal
* **`Q` / `ESC`**: Keluar dari aplikasi

---

## Menjalankan Unit Test

```powershell
# 1. Unit Test Python Inference:
.\.venv\Scripts\python.exe -m unittest discover -s inference/tests -v

# 2. Unit Test Rust GUI Core:
cargo test --manifest-path gui/Cargo.toml -p nopeek-core
```
