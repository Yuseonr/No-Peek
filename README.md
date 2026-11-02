# No-Peek
Sistem Deteksi Pengintip Layar (Shoulder Surfing) Menggunakan Computer Vision Secara Real-Time. Supported on Mac &amp; Windows

## Eksperimen webcam dan video

Tahap ini berupa preview eksperimen OpenCV, mengikuti [rancangan teknis](docs/No-Peek.md).
Eksperimen pertama memakai YOLOv8n-Face sesuai pilihan proyek; keputusan model
akhir masih memerlukan pembandingan dengan baseline YuNet di dokumen.

### Persiapan di macOS

```sh
uv sync
# Bobot yang sudah diunduh:
ls model/yolov8n-face.onnx
```

Dependencies: OpenCV dengan HighGUI, NumPy, ONNX Runtime CPU. Tidak memerlukan
CUDA atau PyTorch. Python lokal dipin ke 3.12 melalui `.python-version`.
Jika kamera gagal dibuka, izinkan aplikasi Terminal/editor yang menjalankan
program di **System Settings > Privacy & Security > Camera**, kemudian buka ulang
terminal. Window dan input keyboard berjalan pada main thread untuk backend Cocoa.

### Webcam

```sh
uv run no-peek --camera 0
# Kamera lain / preview tanpa mirror / eksperimen threshold:
uv run no-peek --camera 1 --no-mirror --confidence 0.6 --threads 2
```

Akuisisi meminta 1280x720, 30 FPS; resolusi/FPS aktual ditampilkan karena kamera
dapat memilih mode berbeda. Preview webcam di-mirror secara default. Data koordinat
dan roll tetap dalam frame asli; label LEFT/RIGHT di preview mengikuti mirror.

### Video player dengan overlay

```sh
uv run no-peek --video "/path/ke/video.mp4"
uv run no-peek --video "/path/ke/video.mov" --metrics experiments/video-01.jsonl
```

Video diproses frame demi frame, tanpa audio. Kecepatan tampilan mengikuti FPS
file sejauh CPU mampu; inference yang lambat memperlambat playback tanpa membuang
frame. Timer risiko memakai timestamp media (`frame_index / fps`), sehingga pause
dan perubahan kecepatan playback tidak memperpanjang durasi risiko. Ini mengasumsikan
video constant frame rate; konversi video variable frame rate ke CFR untuk evaluasi
waktu yang presisi.

| Kontrol | Perilaku |
|---|---|
| `C` | Kalibrasi owner; harus tepat satu wajah terlihat |
| `R` | Reset track, owner, dan timer |
| `M` | Toggle mirror preview |
| `Space` | Pause/resume; di akhir video memulai replay |
| `N` / `B` | Frame berikutnya / sebelumnya (video) |
| `J` / `L` | Mundur/maju 5 detik (video) |
| `0` | Ulang dari awal video |
| `+` / `-` | Kecepatan playback 0.25x sampai 4x |
| Slider `Position` | Seek video |
| `Q` / `Esc` / tutup window | Keluar dan melepaskan sumber |

Klik window preview agar tombol diterima. Seek/back/replay mereset kalibrasi dan
timer supaya episode tidak melintasi lompatan waktu. Pause/resume webcam juga
mereset kalibrasi karena waktu sumber live terus berjalan.

### Data yang terlihat dan diekspor

- Bounding box xyxy, confidence dan lima titik: dua mata, hidung, dua sudut mulut.
- ID track sementara, role owner/observer, posisi, proporsi area wajah, risk timer.
- Status `UNCALIBRATED`, `MONITORING`, `PENDING`, `ALERT` (2s), `ESCALATED` (7s).
- Roll garis mata dalam derajat dan offset hidung relatif terhadap jarak mata.
- FPS pemrosesan (capture+deteksi+analisis, sebelum render/playback wait), waktu
  preprocessing/inference/postprocessing, inference median/p95 pada 300 frame terakhir.
- Panel merinci tiga wajah pertama berdasarkan confidence; seluruh deteksi tetap
  digambar dan diekspor. Detektor membatasi hasil ke 50 wajah per frame.

`--metrics path.jsonl` menyimpan metadata sesi, data tiap frame, event alert,
kalibrasi/reset/seek, serta ringkasan. Box dan landmark memakai piksel frame asli.
File dibuat baru; path yang sudah ada ditolak agar hasil eksperimen tidak tertimpa.
Tidak ada frame, foto, audio, atau identitas biometrik disimpan. Default tanpa log.

**Batas analisis:** model ini belum memberi yaw/pitch atau gaze. Roll berasal dari
garis mata, sedangkan `frontal_proxy` adalah heuristik `abs(nose_offset)<0.3` dan
`abs(roll)<35°`; keduanya belum dikalibrasi sebagai head pose presisi. Wajah observer
harus memiliki area minimal 0.15% frame agar masuk timer. ID menggunakan matching
IoU satu-ke-satu (ambang 0.25), dengan grace period 250ms. Occlusion, perpindahan
cepat, atau wajah saling melintas masih dapat memutus/menukar ID. Hilangnya owner
melewati grace period membatalkan kalibrasi; BR-06 unattended belum diaktifkan karena
tracking spasial tidak membuktikan identitas. Alert muncul di preview dan console
satu kali per transisi; belum berupa toast OS.

Confidence detector adalah skor model, bukan akurasi eksperimen. Event precision,
recall dan false alert/jam memerlukan anotasi ground truth dan belum dihitung oleh
player ini. FPS/latensi harus diukur pada video dan perangkat target; hasil satu
fixture tidak membuktikan target 15–20 FPS atau kualitas shoulder-surfing detection.

### Evaluasi tanpa window

```sh
uv run no-peek --video "/path/video.mp4" --headless --metrics experiments/batch-01.jsonl
uv run no-peek --video "/path/video.mp4" --headless --max-frames 30
uv run python -m unittest discover -s tests -v
```

Mode headless memproses video secepat CPU dan tidak menyediakan kalibrasi interaktif,
sehingga cocok untuk evaluasi detektor/performa. Preview interaktif diperlukan untuk
eksperimen owner dan alert. Keluar dengan Ctrl+C jika perlu.

### Pembagian kode

| File | Input → output |
|---|---|
| `src/no_peek/detector.py` | Frame BGR → deteksi di koordinat asli + waktu tiap tahap |
| `src/no_peek/analysis.py` | Deteksi + timestamp → track, proxy orientasi, timer/event |
| `src/no_peek/source.py` | Kamera/file → frame BGR, index, timestamp dan metadata |
| `src/no_peek/preview.py` | Frame + analisis + statistik → canvas overlay dan panel |
| `src/no_peek/main.py` | Argumen CLI dan keyboard → loop eksperimen + JSONL opsional |

Format model diverifikasi terhadap [implementasi YOLOv8-Face upstream](https://github.com/yakhyo/yolov8-face-onnx-inference).
Bobot yang didukung mengeluarkan tiga feature maps `[1,80,H,W]` (DFL 16 bin,
stride 8/16/32). Bobot YOLO generik/format export lain ditolak dengan pesan format.
Input model dinamis memakai 640x640 sebagai baseline, dengan letterbox agar aspect
ratio terjaga. Review lisensi kode dan bobot upstream sebelum distribusi.
