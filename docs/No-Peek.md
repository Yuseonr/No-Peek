# LAPORAN PENDEKATAN TEKNIS

# Anti-Kepo

**Deteksi Potensi Shoulder Surfing dengan Computer Vision Lokal**

| Informasi | Keterangan |
|---|---|
| Jenis dokumen | Laporan rancangan sistem, pemilihan model, dan rencana eksperimen |
| Target produk | Aplikasi desktop ringan, offline, dapat diinstal di Windows dan macOS |
| Teknologi | Python, OpenCV, YOLOv8n-Face ONNX, MediaPipe Face Landmarker, PySide6 |
| Tanggal | 4 November 2026 |
| Dasar | Dokumen Studi Kasus & Business Rules Anti-Kepo serta implementasi Two-Stage Pipeline |

## Ringkasan keputusan

**Detektor & Head Pose: YOLOv8n-Face ONNX + MediaPipe Face Landmarker.** Sistem menggunakan arsitektur *two-stage pipeline*: YOLOv8n-Face mendeteksi multi-wajah dan 5 *landmark* dasar, diikuti MediaPipe Face Landmarker untuk menghitung matriks transformasi 3D (*yaw*, *pitch*, *roll*). Dilengkapi estimasi jarak fisik (`distance_cm`), auto-kalibrasi *owner* di zona tengah, serta klasifikasi `FRIEND` untuk rekan yang duduk berdampingan.

**Tiga prinsip desain:**

1. **Privasi:** Semua frame diproses lokal tanpa pengiriman ke server.
2. **Gangguan minimal:** Peringatan kecil, tanpa auto-hide, auto-lock, atau mengambil fokus.
3. **Terukur:** Keberhasilan ditentukan oleh *event recall*, *false alert* per jam, latensi, dan biaya komputasi, bukan hanya akurasi detektor.

**Status:** Laporan pendekatan. Tidak ada klaim bahwa model telah lolos pengujian webcam pada perangkat target. Angka benchmark dari proyek lain tidak dinyatakan sebagai hasil Anti-Kepo.

---

## 1. Masalah, tujuan, dan batasan

### 1.1 Masalah yang diselesaikan

Pada laboratorium, perpustakaan, kantin, dan ruang kerja bersama, orang lain dapat melihat konten laptop tanpa diketahui pengguna. Sistem Anti-Kepo dimaksudkan memberi *awareness*, bukan menyimpulkan niat atau identitas orang. Deteksi wajah tambahan saja belum cukup, karena wajah orang lewat dapat menimbulkan *false alarm*. Penelitian PrivacyScout juga menegaskan bahwa wajah pengamat dalam kamera tidak otomatis berarti serangan [4].

### 1.2 Tujuan sistem

1. Mengidentifikasi pengguna utama dan hingga 2 sampai 3 wajah tambahan yang terlihat di webcam.
2. Mengestimasi apakah wajah tambahan cukup mengarah ke area laptop untuk menjadi kandidat risiko.
3. Menampilkan peringatan hanya setelah kondisi bertahan cukup lama, sesuai BR-02 hingga BR-04.
4. Berjalan secara lokal, tanpa identifikasi biometrik atau koneksi cloud saat inferensi.
5. Berjalan pada Windows dan macOS tanpa mengganggu aplikasi kerja utama.

### 1.3 Spesifikasi dan target dari dokumen proyek

| Aspek | Kebutuhan awal |
|---|---|
| Kamera | Webcam internal atau eksternal di atas monitor; FoV sekitar 60 sampai 70 derajat. |
| Jarak | Pengguna utama 40 sampai 80 cm; calon pengamat 80 sampai 250 cm dari area kamera sesuai skenario proyek. |
| Video | Target akuisisi 1280 × 720; 15 sampai 30 FPS. |
| Objek | 1 pengguna utama dan 2 sampai 3 wajah tambahan; wajah sebagian tertutup menjadi kasus uji khusus. |
| Performa | Sasaran awal pemrosesan 15 sampai 20 FPS dan alert dalam 0,5 detik setelah ambang terpenuhi. |
| Interaksi | Toast, tray, atau overlay kecil; tanpa auto-hide dan tanpa mengambil fokus keyboard. |

**Catatan:** 720p adalah target akuisisi, bukan kewajiban menjalankan seluruh model pada 720p. Kecepatan dan tingkat deteksi wajah kecil harus diukur bersama karena *downsampling* dapat menghilangkan wajah pengamat yang jauh. Nilai 85% dari dokumen asli perlu didefinisikan sebagai metrik yang spesifik sebelum menjadi kriteria lulus.

## 2. Keputusan pemilihan model

### 2.1 Mengapa tidak langsung menggunakan YOLO

Tujuan sistem bukan mendeteksi semua objek, tetapi menemukan wajah dengan biaya komputasi kecil. YuNet sudah mengeluarkan *bounding box* dan lima *facial landmarks*. OpenCV Zoo menyediakan bobot ONNX berukuran sekitar 233 KB dan lisensi MIT [1]. Detector YOLO-Face memberi alternatif yang relevan untuk wajah kecil, tetapi model yang lebih berat tidak menjamin *false alarm* lebih rendah jika logika temporal dan orientasi lemah.

| Kandidat | Ukuran model | Peran yang diuji | Keputusan |
|---|---:|---|---|
| **YuNet 2023mar** | Sekitar 233 KB | Detector awal dengan lima landmark | **Baseline wajib**; integrasi paling sederhana. |
| YOLOv8-Lite-s-Face | Sekitar 7,4 MB | Pembanding *face detector* ringan | Uji jika recall YuNet lemah. |
| YOLOv8n-Face | Sekitar 12 MB | Pembanding kemampuan deteksi banyak dan wajah kecil | Uji setelah baseline; implementasi ONNX tersedia. |
| MediaPipe Face Landmarker | Berkas terpisah | Landmark dan matriks transformasi wajah | Tahap tambahan untuk orientasi, bukan pengganti langsung detector. |

*Ukuran di atas adalah ukuran file bobot yang dilaporkan sumber upstream, bukan ukuran RAM atau installer. YuNet dan YOLO memakai sumber dan konfigurasi evaluasi berbeda [1][2][3].*

### 2.2 Kriteria memilih model akhir

Pemilihan berdasarkan data lokal yang sama: **recall wajah pengamat**, **false positive deteksi**, **waktu inferensi median dan p95**, **CPU/RAM**, serta **ukuran akhir aplikasi**. Untuk setiap kandidat, tetapkan resolusi input, *confidence threshold*, platform, dan jumlah *threads* sebelum membandingkan. Jangan mencampur AP WIDER FACE upstream dengan hasil pengujian Anti-Kepo.

## 3. Arsitektur sistem yang diusulkan

```mermaid
flowchart TD
    A["Webcam / Video lokal<br/>Pengambilan frame 720p (source.py)"]
    B["Tahap 1: Detektor wajah<br/>YOLOv8n-Face ONNX + C++ NMS (detector.py)"]
    C["Tahap 2: Head Pose 3D<br/>MediaPipe Face Landmarker yaw/pitch/roll (pose.py)"]
    D["Geometri spasial & jarak<br/>Estimasi distance_cm & deteksi berdampingan (geometry.py)"]
    E["Auto-kalibrasi OWNER, klasifikasi FRIEND/OBSERVER,<br/>& Risk Engine Temporal 2s/7s (analysis.py)"]
    F["Preview HighGUI / System tray & toast non-intrusif<br/>(preview.py / GUI)"]
    A --> B --> C --> D --> E --> F
```

*Gambar 1. Arsitektur modular Anti-Kepo (No-Peek). Seluruh pemrosesan frame berlangsung pada perangkat pengguna.*

### 3.1 Peran setiap modul

- **Acquisition (`source.py`):** OpenCV `VideoCapture` mengambil frame dari webcam atau file video, serta mengelola jam monotonik/media.
- **Detection (`detector.py`):** YOLOv8n-Face ONNX menghasilkan $(x_1,y_1,x_2,y_2)$, skor keyakinan, dan lima landmark dengan C++ NMS (`cv2.dnn.NMSBoxes`).
- **Head Pose 3D (`pose.py`):** MediaPipe Face Landmarker menghitung matriks transformasi 3D wajah pada *square crop* untuk mengekstrak sudut `yaw_deg`, `pitch_deg`, dan `roll_deg`.
- **Spatial Geometry (`geometry.py`):** Menghitung estimasi jarak kamera ke wajah (`distance_cm`) menggunakan *Pinhole Camera Model* terkoreksi sudut *yaw*, memeriksa zona tengah, dan mengevaluasi apakah wajah lain duduk berdampingan (`FRIEND`).
- **Owner Tracking & Temporal Engine (`analysis.py`):** Mengelola auto-kalibrasi `OWNER` saat pengguna duduk stabil di zona tengah ($\ge 0{,}6\text{ s}$) maupun kalibrasi manual (`C`), membedakan `OWNER`, `FRIEND`, dan `OBSERVER`, serta menjalankan aturan waktu peringatan 2 detik (`ALERT`) dan 7 detik (`ESCALATED`).
- **Notification / Preview (`preview.py`):** Menampilkan antarmuka visual dan panel telemetri (serta lapisan notifikasi desktop non-intrusif pada tahap GUI).

### 3.2 Kesalahan yang harus diantisipasi

- **Owner swap:** Wajah orang lain lebih besar atau lebih dekat ke pusat frame. Solusi: kalibrasi owner dan pelacakan antar-frame, bukan memilih wajah terbesar pada setiap frame.
- **Missed detection:** Blur atau oklusi memutus track. Solusi: uji toleransi kehilangan sekitar 200 sampai 300 ms sebelum reset timer.
- **Small face:** Pengamat jauh kehilangan fitur ketika frame diperkecil. Solusi: uji 640 dan 1280 input, lalu sesuaikan detector.
- **FoV:** Orang di luar kamera mustahil diketahui; tidak boleh disebut status aman secara absolut.

## 4. Logika aturan bisnis

| Rule | Kondisi utama | Ambang | Aksi yang dirancang |
|---|---|---|---|
| BR-01 | Tidak ada kandidat yang memenuhi risiko (hanya `OWNER` / `FRIEND`) | Kontinu | Status aman atau `MONITORING`; tanpa alert. |
| BR-02 | `OBSERVER` lewat atau belum menetap | < 2 s | Status `PENDING`; timer belum menghasilkan notifikasi. |
| BR-03 | `OBSERVER` stabil, orientasi 3D memenuhi syarat | ≥ 2 s | Status `ALERT`; tampilkan indikator kiri/tengah/kanan. |
| BR-04 | `OBSERVER` berisiko tetap menghadap layar | ≥ 7 s | Status `ESCALATED`; eskalasi visual halus, bunyi/log opsional. |
| BR-05 | Wajah menyamping ($|\text{yaw}| \ge 35^\circ$) atau menunduk/mendongak ($|\text{pitch}| \ge 25^\circ$) | Kontinu | Abaikan sebagai kandidat risiko; timer direset ke 0. |
| BR-06 | Owner tidak terlihat dan wajah asing muncul | ≥ 3 s | Alert *unattended screen*; butuh validasi khusus. |
| BR-07 | Wajah kedua duduk berdampingan (*side-by-side*) sejajar dengan `OWNER` | Kontinu | Klasifikasikan sebagai `FRIEND`; abaikan dari perhitungan risiko. |

*Ringkasan BR-01 hingga BR-07 berdasarkan dokumen proyek dan pengembangan aturan spasial.*

### 4.1 Orientasi, estimasi jarak, dan klasifikasi `FRIEND`

Panjang fokus kamera ($f_{\text{px}}$) dihitung dari lebar frame $W$ dan *Field of View* horizontal ($\text{FoV} \approx 60^\circ$):

$$f_{\text{px}} = \frac{W / 2}{\tan(\text{FoV} / 2)}$$

Estimasi jarak wajah ke kamera ($\text{distance\_cm}$) menggabungkan lebar kotak wajah ($W_{\text{face}} \approx 15\text{ cm}$) dan jarak piksel antar-mata ($\text{IPD} \approx 6{,}3\text{ cm}$) yang dikompensasi terhadap penyempitan perspektif saat kepala menoleh sebesar sudut $\text{yaw}$:

$$d_{\text{eyes, corr}} = \frac{d_{\text{eyes}}}{\max(\cos(|\text{yaw}|), 0{,}5)}, \quad \text{distance\_cm} = 0{,}6\left(\frac{15{,}0 \cdot f_{\text{px}}}{w_i}\right) + 0{,}4\left(\frac{6{,}3 \cdot f_{\text{px}}}{d_{\text{eyes, corr}}}\right)$$

Wajah non-owner diklasifikasikan sebagai **`FRIEND`** (bukan `OBSERVER`) apabila berada dalam jarak duduk meja ($\le 115\text{ cm}$), bersebelahan secara horizontal dengan `OWNER`, memiliki ketinggian vertikal yang sejajar duduk (tidak melongok dari atas pundak), serta memiliki jarak sejajar ($\le d_{\text{owner}} + 25\text{ cm}$ atau toleransi ukuran kepala lebih mungil hingga $1{,}65\times d_{\text{owner}}$ saat duduk sejajar).

Untuk UI, posisi horizontal wajah dihitung dari pusat:

$$c_{x,i} = x_i + \frac{w_i}{2}$$

- **Kiri:** $c_x < W/3$.
- **Tengah:** $W/3 \leq c_x \leq 2W/3$.
- **Kanan:** $c_x > 2W/3$.

Posisi ini relatif terhadap frame kamera; pada preview yang di-*mirror*, label harus konsisten dengan tampilan pengguna.

Sudut orientasi kepala 3D (*yaw*, *pitch*, *roll*) diekstrak dari matriks transformasi $4 \times 4$ MediaPipe Face Landmarker melalui dekomposisi $R = R_y(\text{yaw}) R_x(\text{pitch}) R_z(\text{roll})$ dengan ambang default $|\text{yaw}| < 35^\circ$, $|\text{pitch}| < 25^\circ$, dan $|\text{roll}| < 35^\circ$ (fallback ke *5-point proxy* apabila *mesh* tidak terdeteksi).

### 4.2 Perilaku temporal yang disarankan

Untuk setiap calon pengamat, simpan ID pelacakan, waktu awal risiko, waktu terakhir terlihat, serta penanda alert dan eskalasi. Gunakan jam monotonik, bukan jumlah frame.

```mermaid
flowchart LR
    A[SAFE] -->|Kandidat muncul| B[PENDING]
    B -->|Risiko ≥ 2 s| C[ALERT]
    C -->|Risiko ≥ 7 s total| D[ESCALATED]
    B -->|Hilang atau tidak memenuhi syarat| A
```

Satu observer yang hilang sesaat tidak langsung dianggap orang baru; gunakan *grace period* yang diuji. Setelah risiko hilang secara stabil, episode ditutup. Alert dipicu sekali per episode agar tidak terjadi spam.

## 5. Status implementasi saat ini dan cara mereproduksi

### 5.1 Status pembuktian pada repositori saat ini

| Komponen | Status | Keterangan |
|---|---|---|
| Detektor YOLOv8n-Face ONNX (`detector.py`) | **Tersedia** | Deteksi multi-wajah, 5 landmark, `cv2.dnn.blobFromImage` & C++ NMS `cv2.dnn.NMSBoxes`. |
| Head Pose 3D MediaPipe (`pose.py`) | **Tersedia** | Ekstraksi sudut `yaw_deg`, `pitch_deg`, `roll_deg` dari matriks transformasi 3D wajah. |
| Estimasi jarak & Geometri `FRIEND` (`geometry.py`) | **Tersedia** | Estimasi `distance_cm` terkoreksi *yaw* dan pembedaan `FRIEND` (berdampingan) vs `OBSERVER`. |
| Auto-kalibrasi & Temporal Engine (`analysis.py`) | **Tersedia** | Auto-kalibrasi `OWNER` di zona tengah ($\ge 0{,}6\text{ s}$), *grace period* 250 ms, event 2 s (`ALERT`) dan 7 s (`ESCALATED`). |
| Pengujian unit & integrasi (`tests/`) | **Tersedia (16 test lulus)** | Mencakup uji decoder, matriks 3D, jarak, auto-kalibrasi, role `FRIEND`, dan kontrol video. |
| Aplikasi GUI Desktop & Installer | **Tahap berikutnya** | Persiapan *scaffold* aplikasi GUI dan pengemasan lintas platform. |

### 5.2 Menjalankan sistem

```powershell
# Sinkronisasi environment dengan uv
uv sync

# Menjalankan dengan webcam lokal
uv run no-peek --camera 0

# Menjalankan unit test
.\.venv\Scripts\python.exe -m unittest discover -s tests -v
```

Kontrol saat jendela preview aktif:
- **`A`**: *Toggle* Auto-Kalibrasi `OWNER` (`ON`/`OFF`).
- **`C`**: Kalibrasi `OWNER` manual saat tepat satu wajah terlihat.
- **`R`**: Reset pelacakan dan kalibrasi.
- **`M`**: *Toggle* mode cermin (*mirror*).
- **`Q` / `ESC`**: Keluar dari program.

## 6. Metodologi eksperimen

### Catatan implementasi pada checkout ini

Implementasi pada repositori ini menggunakan pipeline dua tahap: **YOLOv8n-Face ONNX** (`model/yolov8n-face.onnx`) sebagai detektor wajah utama dan **MediaPipe Face Landmarker** (`model/face_landmarker.task`) untuk estimasi orientasi kepala 3D (`yaw`, `pitch`, `roll`), dengan *fallback* otomatis ke *5-point proxy* bila *mesh* wajah tidak terdeteksi.

Entry point aplikasi adalah `uv run no-peek` (`no_peek.main:main`). Sumber input dapat berupa webcam (`--camera 0`) atau file video (`--video path.mp4`), dilengkapi estimasi jarak wajah (`distance_cm`), auto-kalibrasi `OWNER` di zona tengah layar, klasifikasi `FRIEND` untuk wajah yang duduk berdampingan, serta pencatatan telemetri JSONL opsional (`--metrics`). Detail lengkap argumen CLI dan struktur modul tersedia di [README](../README.md).

### 6.1 Desain pengujian

Uji tiga tahapan secara berurutan supaya waktu tidak habis pada kombinasi yang belum diperlukan.

1. **Detektor:** YuNet terlebih dahulu. Uji YOLO-Face hanya jika recall wajah kecil pada data Anti-Kepo tidak mencukupi atau jika ada *trade-off* performa yang layak.
2. **Orientasi:** Bandingkan *face count* vs proxy lima landmark vs head pose. Targetnya mengurangi peringatan pada orang yang mengobrol atau menoleh, bukan meningkatkan AP detector saja.
3. **Temporal:** Bandingkan alert satu frame vs syarat 2 detik vs 2 detik dengan tracking dan *grace period*. Catat false alert per jam dan waktu respons.

### 6.2 Skenario dan label data

| ID | Adegan | Label kejadian dan hasil yang diharapkan |
|---|---|---|
| S1 | Satu orang memakai laptop | Negatif; tanpa alert. |
| S2 | Orang lewat kurang dari 2 s | Negatif; tanpa alert. |
| S3 | Orang berdiri menghadap layar ≥ 2 s | Positif; BR-03 dan posisi tepat. |
| S4 | Orang mengobrol atau menoleh | Negatif; filter orientasi bekerja. |
| S5 | Orang diam di belakang ≥ 7 s | Positif; BR-04 tepat sekali. |
| S6 | Pengguna keluar dan orang lain mendekat | Khusus BR-06; evaluasi terpisah. |
| S7 | Pengamat jauh, wajah kecil, cahaya redup | Uji sensitivitas detector dan false negative. |
| S8 | Pengamat sebagian tertutup / pengguna bergerak | Uji kestabilan tracking, tidak swap owner. |

Rekam video uji dengan persetujuan partisipan, kemudian beri anotasi *timestamp* awal dan akhir per kejadian, bounding box wajah dan kategori orientasi bila diperlukan. Pisahkan sesi atau subjek, bukan frame acak, jika melakukan validasi model terlatih. Dataset tidak perlu diunggah ke layanan eksternal.

### 6.3 Metrik yang dilaporkan

**Event recall:**

$$\text{Event recall} = \frac{TP_{\mathrm{event}}}{TP_{\mathrm{event}}+FN_{\mathrm{event}}}$$

**Event precision:**

$$\text{Event precision} = \frac{TP_{\mathrm{event}}}{TP_{\mathrm{event}}+FP_{\mathrm{event}}}$$

**False alerts per hour:**

$$\text{False alerts/hour} = \frac{N_{\mathrm{alert\ salah}}}{T_{\mathrm{uji\ negatif}}\;(\mathrm{jam})}$$

**Alert delay:**

$$\text{Alert delay} = t_{\mathrm{alert}}-(t_{\mathrm{start}}+2\,\mathrm{s})$$

*TP event* berarti sistem mengeluarkan satu alert pada jendela toleransi yang telah ditetapkan untuk kejadian positif. Cantumkan definisi toleransi itu sebelum pengujian. Selain metrik event, catat recall wajah kecil, median/p95 *inference time*, FPS efektif, penggunaan CPU/RAM, dan konsumsi baterai bila memungkinkan.

### 6.4 Target penerimaan awal

| Tolok ukur | Kriteria keputusan sementara |
|---|---|
| Deteksi wajah | Sasaran proyek ≥ 85% untuk kemampuan deteksi multi-face; tetapkan rumus operasional saat evaluasi. |
| BR-02 dan BR-05 | Tidak boleh memicu alert pada skenario S2/S4 yang tervalidasi. |
| Latensi alert | ≤ 0,5 s setelah syarat risiko 2 s terpenuhi (target proyek). |
| Kecepatan | Sasaran 15 sampai 20 FPS dari dokumen; nilai pemrosesan model yang lebih rendah dapat diuji jika deteksi tetap stabil dan latensi terpenuhi. |
| Stabilitas | Tidak ada spam alert dan tidak terjadi owner swap pada percobaan standar. |

Jangan mengklaim pemenuhan target hanya berdasarkan satu demo. Ulangi setiap skenario pada perangkat dan kondisi cahaya berbeda, laporkan jumlah percobaan, dan sertakan hasil negatif.

## 7. Kajian riset dan adaptasi

| Penelitian atau implementasi | Pelajaran yang diadaptasi |
|---|---|
| *PrivacyScout* (2022) [4] | Wajah dalam FoV bukan bukti serangan. Lokasi, sudut, jarak, dan jenis informasi memengaruhi risiko. Anti-Kepo perlu filter spasial dan orientasi. |
| *ShouldAR* (2024) [5] | Penggunaan informasi gaze dapat meningkatkan analisis shoulder surfing. Riset memakai AR dan studi 24 peserta dengan deteksi 87,28%, tetapi angka tersebut tidak dapat dipindahkan ke webcam laptop. |
| Brudy dkk. (2014) [6] | *Awareness* berbasis kedekatan dan orientasi pengguna/pengamat relevan untuk UI yang tidak mengganggu. |
| *peeker* (repo desktop) [7] | Contoh integrasi MediaPipe, tracking posisi, dan state alert; threshold sebagai kandidat awal. |
| *EyesOff ShoulderGuard* (blog teknis) [8] | Desain dua tahap, YuNet diikuti classifier orientasi; pemrosesan sebagian frame untuk menekan konsumsi daya. Angka performa adalah laporan pengembang, belum validasi Anti-Kepo. |

**Implikasi:** Pendekatan paling realistis adalah *two-stage pipeline*. Jalankan detektor kecil untuk semua frame yang diproses; jalankan estimasi orientasi hanya pada wajah non-owner yang lolos filter posisi. Jangan langsung melatih classifier khusus tanpa bukti bahwa aturan orientasi tidak cukup.

## 8. Rencana aplikasi desktop dan privasi

### 8.1 Stack dan pengemasan

| Lapisan | Teknologi | Alasan |
|---|---|---|
| Capture dan deteksi | OpenCV + YOLOv8n-Face ONNX (`onnxruntime`) | Deteksi wajah & 5 landmark akurat tanpa dependensi PyTorch. |
| Head Pose 3D | MediaPipe Tasks (`FaceLandmarker`) | Matriks transformasi wajah 3D untuk sudut `yaw`, `pitch`, dan `roll`. |
| Geometri & State/rules | Python + NumPy (`geometry.py`, `analysis.py`) | Estimasi jarak `distance_cm`, auto-kalibrasi, role `FRIEND`, dan mudah diuji secara unit. |
| Desktop GUI | PySide6 | System tray, toast/overlay, kontrol start/stop. |
| Packaging | PyInstaller | Build bundle untuk Windows dan macOS secara terpisah [9]. |

### 8.2 Prinsip deployment

- Paket aplikasi memuat bobot model lokal sehingga *runtime* tidak perlu Internet. Jangan menambahkan PyTorch ke installer jika inferensi dapat ditangani OpenCV DNN atau ONNX Runtime.
- **Windows:** Uji build pada OS Windows sesungguhnya, izin webcam, *system tray*, dan startup opsional.
- **macOS:** Tambahkan `NSCameraUsageDescription` ke metadata bundle, lalu uji izin kamera pada perangkat Apple Silicon dan Intel [10].
- Uji instalasi bersih, perilaku offline, penghentian proses kamera, dan apakah overlay merebut fokus keyboard.
- **Lisensi:** YuNet dari OpenCV Zoo dilisensikan MIT; periksa lisensi setiap bobot YOLO-Face dan kewajiban lisensi PySide6 sebelum distribusi [1][11][12].

### 8.3 Privasi dan keamanan data

Pemrosesan frame tetap di memori. Tidak ada pengenalan identitas atau pengunggahan gambar. Dokumen proyek meminta thumbnail dan snapshot pada BR-04/BR-06, tetapi penyimpanan bukti wajah menambah risiko privasi. Karena itu, **rekomendasi implementasi** adalah thumbnail hanya ditampilkan sementara dan penyimpanan snapshot **nonaktif secara default**, dengan persetujuan eksplisit, lokasi penyimpanan lokal, retensi pendek, dan kontrol hapus. Catatan ini merupakan penyesuaian desain, bukan fitur yang sudah selesai.

## 9. Rencana pengerjaan dan keputusan akhir

| Fase | Keluaran | Syarat untuk lanjut |
|---|---|---|
| F1 | Jalankan YuNet POC, rekam metrik 8 skenario | Webcam stabil, data evaluasi dapat diulang. |
| F2 | Benchmark YOLO-Face bila diperlukan | Ada bukti peningkatan recall yang sepadan dengan biaya komputasi. |
| F3 | Tambahkan owner tracking dan head pose | False alert turun tanpa banyak kehilangan event recall. |
| F4 | Implementasi tray, toast, setting, opt-in log | Tidak ada fokus keyboard terganggu; lokal/offline. |
| F5 | Build Windows + macOS dan uji instalasi | Izin kamera, performa, dan startup/stop lulus uji. |

**Rekomendasi final:** Mulai dengan **YuNet + tracking spasial + aturan temporal**. Gunakan MediaPipe bila proxy orientasi gagal mengurangi false alarm. Gunakan YOLO-Face hanya apabila pengujian membuktikan bahwa recall wajah kecil menjadi kendala signifikan. Prioritaskan validasi risiko per kejadian dan kualitas notifikasi dibanding menambah model sebelum masalahnya terukur.

---

## Referensi dan sumber teknis

1. OpenCV Zoo. *YuNet Face Detection: README, model, dan lisensi MIT*. https://github.com/opencv/opencv_zoo/tree/main/models/face_detection_yunet. Diakses 2 November 2026.
2. Valikhujaev, Y. *YOLOv8-Face ONNX Inference*. https://github.com/yakhyo/yolov8-face-onnx-inference. Diakses 2 November 2026.
3. Google AI Edge. *Face Landmark Detection Guide for Python*. https://developers.google.com/edge/mediapipe/solutions/vision/face_landmarker/python. Diakses 2 November 2026.
4. Bâce, M., Saad, A., Khamis, M., Schneegass, S., dan Bulling, A. (2022). *PrivacyScout: Assessing Vulnerability to Shoulder Surfing on Mobile Devices*. PoPETs 2022(3), 650–669. https://doi.org/10.56553/popets-2022-0090.
5. Corbett, M., David-John, B., Shang, J., dan Ji, B. (2024). *ShouldAR: Detecting Shoulder Surfing Attacks Using Multimodal Eye Tracking and Augmented Reality*. Proc. ACM IMWUT 8(3). https://doi.org/10.1145/3678573.
6. Brudy, F., Ledo, D., Greenberg, S., dan Butz, A. (2014). *Is Anyone Looking? Mitigating Shoulder Surfing on Public Displays through Awareness and Protection*. Pervasive Displays. https://fbrudy.net/projects/mediating-privacy.
7. silhovette. *peeker: Local-only Windows Privacy Watcher*. https://github.com/silhovette/peeker. Diakses 2 November 2026.
8. Mohammad, Y. (2026). *How EyesOff Spots Shoulder Surfers Using a Privacy-First Approach*. https://www.eyesoff.app/blog/how-eyesoff-spots-shoulder-surfers/.
9. PyInstaller. *PyInstaller Manual*. https://www.pyinstaller.org/en/stable/. Diakses 2 November 2026.
10. Apple Developer. *NSCameraUsageDescription*. https://developer.apple.com/documentation/bundleresources/information-property-list/nscamerausagedescription.
11. Ultralytics. *Licensing (AGPL-3.0 dan Enterprise)*. https://www.ultralytics.com/license.
12. Qt. *Qt Licensing*. https://doc.qt.io/qt-6/licensing.html.

**Sumber internal proyek:** `PROYEK.txt` (Dokumen Studi Kasus & Business Rules Anti-Kepo) serta file `antikepo_yunet.py` dan README yang disusun untuk eksperimen awal. Seluruh threshold dokumen dan status implementasi merujuk pada sumber internal tersebut, kecuali jika dinyatakan sebagai rekomendasi.
