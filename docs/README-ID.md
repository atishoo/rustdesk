<p align="center">
  <img src="../res/logo-header.svg" alt="capylink - Your remote desktop"><br>
  <a href="#free-public-servers">Servers</a> •
  <a href="#raw-steps-to-build">Build</a> •
  <a href="#how-to-build-with-docker">Docker</a> •
  <a href="#file-structure">Structure</a><br>
  [<a href="../README.md">English</a>] | [<a href="README-UA.md">Українська</a>] | [<a href="README-CS.md">česky</a>] | [<a href="README-ZH.md">中文</a>] | [<a href="README-HU.md">Magyar</a>] | [<a href="README-ES.md">Español</a>] | [<a href="README-FA.md">فارسی</a>] | [<a href="README-FR.md">Français</a>] | [<a href="README-DE.md">Deutsch</a>] | [<a href="README-PL.md">Polski</a>] | [<a href="README-FI.md">Suomi</a>] | [<a href="README-ML.md">മലയാളം</a>] | [<a href="README-JP.md">日本語</a>] | [<a href="README-NL.md">Nederlands</a>] | [<a href="README-IT.md">Italiano</a>] | [<a href="README-RU.md">Русский</a>] | [<a href="README-PTBR.md">Português (Brasil)</a>] | [<a href="README-EO.md">Esperanto</a>] | [<a href="README-KR.md">한국어</a>] | [<a href="README-AR.md">العربي</a>] | [<a href="README-VN.md">Tiếng Việt</a>] | [<a href="README-GR.md">Ελληνικά</a>]<br>
  <b>Kami membutuhkan bantuanmu untuk menterjemahkan file README dan <a href="https://github.com/atishoo/rustdesk/tree/ultrapro/src/lang">capylink UI</a> ke Bahasa Indonesia</b>
</p>

> [!Caution]
> **Peringatan Penyalahgunaan:** <br>
> Pengembang capylink tidak membenarkan atau mendukung penggunaan perangkat lunak ini yang tidak etis atau melanggar hukum. Penyalahgunaan, seperti akses tanpa izin, pengambilalihan kendali, atau pelanggaran privasi, sangat dilarang berdasarkan pedoman kami. Para pengembang tidak bertanggung jawab atas penyalahgunaan aplikasi ini.


Merupakan perangkat lunak Remote Desktop yang baru, dan dibangun dengan Rust. Bahkan kamu bisa langsung menggunakannya tanpa perlu melakukan konfigurasi tambahan. Serta memiliki kontrol penuh terhadap semua data, tanpa perlu merasa was-was tentang isu keamanan, dan yang lebih menarik adalah memiliki opsi untuk menggunakan server rendezvous/relay milik kami, [konfigurasi server sendiri](https://rustdesk.com/server), atau [tulis rendezvous/relay server anda sendiri](https://github.com/rustdesk/rustdesk-server-demo).


capylink mengajak semua orang untuk ikut berkontribusi. Lihat [`docs/CONTRIBUTING-ID.md`](CONTRIBUTING-ID.md) untuk melihat panduan.

[**FAQ**](https://github.com/rustdesk/rustdesk/wiki/FAQ)

[**UNDUH BINARY**](https://github.com/atishoo/rustdesk/releases/tag/ultrapro)


## Dependensi

Pada versi desktop, antarmuka pengguna (GUI) menggunakan [Sciter](https://sciter.com/) atau flutter

Kamu bisa mengunduh Sciter dynamic library disini.

[Windows](https://raw.githubusercontent.com/c-smile/sciter-sdk/master/bin.win/x64/sciter.dll) |
[Linux](https://raw.githubusercontent.com/c-smile/sciter-sdk/master/bin.lnx/x64/libsciter-gtk.so) |
[MacOS](https://raw.githubusercontent.com/c-smile/sciter-sdk/master/bin.osx/libsciter.dylib)

## Langkah awal untuk memulai

- Siapkan env development Rust dan env build C++

- Install [vcpkg](https://github.com/microsoft/vcpkg), dan atur variabel env `VCPKG_ROOT` dengan benar

  - Windows: vcpkg install libvpx:x64-windows-static libyuv:x64-windows-static opus:x64-windows-static aom:x64-windows-static
  - Linux/MacOS: vcpkg install libvpx libyuv opus aom

- jalankan `cargo run`

## [Build](https://rustdesk.com/docs/en/dev/build/)

## Cara Build di Linux

### Ubuntu 18 (Debian 10)

```sh
sudo apt install -y zip g++ gcc git curl wget nasm yasm libgtk-3-dev clang libxcb-randr0-dev libxdo-dev \
        libxfixes-dev libxcb-shape0-dev libxcb-xfixes0-dev libasound2-dev libpulse-dev cmake make \
        libclang-dev ninja-build libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev
```

### Fedora 28 (CentOS 8)

```sh
sudo yum -y install gcc-c++ git curl wget nasm yasm gcc gtk3-devel clang libxcb-devel libxdo-devel libXfixes-devel pulseaudio-libs-devel cmake alsa-lib-devel
```

### Arch (Manjaro)

```sh
sudo pacman -Syu --needed unzip git cmake gcc curl wget yasm nasm zip make pkg-config clang gtk3 xdotool libxcb libxfixes alsa-lib pipewire
```

### Install vcpkg

```sh
git clone https://github.com/microsoft/vcpkg
cd vcpkg
git checkout 2023.04.15
cd ..
vcpkg/bootstrap-vcpkg.sh
export VCPKG_ROOT=$HOME/vcpkg
vcpkg/vcpkg install libvpx libyuv opus aom
```

### Mengatasi masalah libvpx (Untuk Fedora)

```sh
cd vcpkg/buildtrees/libvpx/src
cd *
./configure
sed -i 's/CFLAGS+=-I/CFLAGS+=-fPIC -I/g' Makefile
sed -i 's/CXXFLAGS+=-I/CXXFLAGS+=-fPIC -I/g' Makefile
make
cp libvpx.a $HOME/vcpkg/installed/x64-linux/lib/
cd
```

### Build

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env
git clone --branch ultrapro https://github.com/atishoo/rustdesk capylink
cd capylink
mkdir -p target/debug
wget https://raw.githubusercontent.com/c-smile/sciter-sdk/master/bin.lnx/x64/libsciter-gtk.so
mv libsciter-gtk.so target/debug
VCPKG_ROOT=$HOME/vcpkg cargo run
```

## Cara Build dengan Docker

Mulailah dengan melakukan kloning (clone) repositori dan build dengan docker container:

```sh
git clone --branch ultrapro https://github.com/atishoo/rustdesk capylink
cd capylink
docker build -t "capylink-builder" .
```

Selanjutnya, setiap kali ketika kamu akan melakukan build aplikasi, jalankan perintah berikut:

```sh
docker run --rm -it -v $PWD:/home/user/capylink -v capylink-git-cache:/home/user/.cargo/git -v capylink-registry-cache:/home/user/.cargo/registry -e PUID="$(id -u)" -e PGID="$(id -g)" capylink-builder
```

Perlu diingat bahwa pada saat build pertama kali, mungkin memerlukan waktu lebih lama sebelum dependensi di-cache, build berikutnya akan lebih cepat. Selain itu, jika perlu menentukan argumen yang berbeda untuk perintah build, kamu dapat melakukannya di akhir perintah di posisi `<OPTIONAL-ARGS>`. Misalnya, jika ingin membangun versi rilis yang dioptimalkan, jalankan perintah di atas dan tambahkan `--release`. Hasil eksekusi perintah tersebut akan tersimpan pada target folder di sistem kamu, dan dapat dijalankan dengan:

```sh
target/debug/capylink
```

Atau, jika kamu menjalankan rilis yang dapat dieksekusi:

```sh
target/release/capylink
```

Harap pastikan bahwa kamu menjalankan perintah ini dari repositori root capylink, jika tidak demikian, aplikasi mungkin tidak dapat menemukan sumber yang diperlukan. Dan juga, perintah cargo seperti `install` atau `run` saat ini tidak didukung melalui metode ini karena, proses menginstal atau menjalankan program terjadi di dalam container bukan pada host.

## Struktur File

- **[libs/hbb_common](https://github.com/atishoo/rustdesk/tree/ultrapro/libs/hbb_common)**: video codec, config, tcp/udp wrapper, protobuf, fs functions untuk transfer file, dan beberapa fungsi utilitas lainnya
- **[libs/scrap](https://github.com/atishoo/rustdesk/tree/ultrapro/libs/scrap)**: screen capture
- **[libs/enigo](https://github.com/atishoo/rustdesk/tree/ultrapro/libs/enigo)**: spesifikasi platform keyboard/mouse control
- **[src/ui](https://github.com/atishoo/rustdesk/tree/ultrapro/src/ui)**: GUI
- **[src/server](https://github.com/atishoo/rustdesk/tree/ultrapro/src/server)**: audio/clipboard/input/video services, dan network connections
- **[src/client.rs](https://github.com/atishoo/rustdesk/tree/ultrapro/src/client.rs)**: start a peer connection
- **[src/rendezvous_mediator.rs](https://github.com/atishoo/rustdesk/tree/ultrapro/src/rendezvous_mediator.rs)**: Komunikasi dengan [capylink-server](https://github.com/atishoo/rustdesk-server/tree/ultrapro), menunggu untuk remote direct (TCP hole punching) atau relayed connection
- **[src/platform](https://github.com/atishoo/rustdesk/tree/ultrapro/src/platform)**: kode khusus platform
- **[flutter](https://github.com/atishoo/rustdesk/tree/ultrapro/flutter)**: kode Flutter untuk platform desktop dan perangkat seluler
