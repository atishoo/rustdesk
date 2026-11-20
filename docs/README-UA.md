<p align="center">
  <img src="../res/logo-header.svg" alt="capylink - Ваш віддалений робочий стіл"><br>
  <a href="#кроки-для-збірки">Збірка</a> •
  <a href="#як-зібрати-за-допомогою-docker">Docker</a> •
  <a href="#структура-файлів">Структура</a><br>
  [<a href="../README.md">English</a>] | [<a href="README-CS.md">česky</a>] | [<a href="README-ZH.md">中文</a>] | [<a href="README-HU.md">Magyar</a>] | [<a href="README-ES.md">Español</a>] | [<a href="README-FA.md">فارسی</a>] | [<a href="README-FR.md">Français</a>] | [<a href="README-DE.md">Deutsch</a>] | [<a href="README-PL.md">Polski</a>] | [<a href="README-ID.md">Indonesian</a>] | [<a href="README-FI.md">Suomi</a>] | [<a href="README-ML.md">മലയാളം</a>] | [<a href="README-JP.md">日本語</a>] | [<a href="README-NL.md">Nederlands</a>] | [<a href="README-IT.md">Italiano</a>] | [<a href="README-RU.md">Русский</a>] | [<a href="README-PTBR.md">Português (Brasil)</a>] | [<a href="README-EO.md">Esperanto</a>] | [<a href="README-KR.md">한국어</a>] | [<a href="README-AR.md">العربي</a>] | [<a href="README-VN.md">Tiếng Việt</a>] | [<a href="README-DA.md">Dansk</a>] | [<a href="README-GR.md">Ελληνικά</a>] | [<a href="README-TR.md">Türkçe</a>] | [<a href="README-NO.md">Norsk</a>] | [<a href="README-RO.md">Română</a>]<br>
  <b>Нам потрібна ваша допомога з перекладом цього README, <a href="https://github.com/atishoo/rustdesk/tree/ultrapro/src/lang">інтерфейсу capylink</a> та <a href="https://github.com/rustdesk/doc.rustdesk.com">документації capylink</a> вашою рідною мовою</b>
</p>

> [!CAUTION]
> **Застереження щодо неналежного використання:** <br>
> Розробники capylink не схвалюють і не підтримують неетичне або незаконне використання цього програмного забезпечення. Неналежне використання, зокрема несанкціонований доступ, керування або втручання в приватність, суворо суперечить нашим правилам. Автори не несуть відповідальності за неналежне використання застосунку.


capylink — ще одне рішення для віддаленого робочого столу, написане на Rust. Працює одразу після запуску й не потребує налаштування. Ви повністю контролюєте свої дані та їхню безпеку. Можна використовувати наш rendezvous/relay-сервер, [налаштувати власний](https://rustdesk.com/server) або [написати власний rendezvous/relay-сервер](https://github.com/rustdesk/rustdesk-server-demo).


capylink вітає внески від усіх охочих. Дивіться [CONTRIBUTING.md](CONTRIBUTING.md), щоб дізнатися, з чого почати.

[**FAQ**](https://github.com/rustdesk/rustdesk/wiki/FAQ)

[**ЗАВАНТАЖИТИ ГОТОВУ ЗБІРКУ**](https://github.com/atishoo/rustdesk/releases/tag/ultrapro)


## Залежності

Версії для настільних ОС використовують Flutter або Sciter (застарілий) для графічного інтерфейсу. Ця інструкція стосується лише Sciter, оскільки з ним простіше почати. Для збірки версії на Flutter дивіться наш [CI](https://github.com/atishoo/rustdesk/blob/ultrapro/.github/workflows/flutter-build.yml).

Динамічну бібліотеку Sciter потрібно завантажити окремо.

[Windows](https://raw.githubusercontent.com/c-smile/sciter-sdk/master/bin.win/x64/sciter.dll) |
[Linux](https://raw.githubusercontent.com/c-smile/sciter-sdk/master/bin.lnx/x64/libsciter-gtk.so) |
[macOS](https://raw.githubusercontent.com/c-smile/sciter-sdk/master/bin.osx/libsciter.dylib)

## Кроки для збірки

- Підготуйте середовище розробки Rust і середовище для збирання C++.

- Встановіть [vcpkg](https://github.com/microsoft/vcpkg) і правильно задайте змінну середовища `VCPKG_ROOT`.

  - Windows: vcpkg install libvpx:x64-windows-static libyuv:x64-windows-static opus:x64-windows-static aom:x64-windows-static
  - Linux/macOS: vcpkg install libvpx libyuv opus aom

- Запустіть `cargo run`.

## [Збирання](https://rustdesk.com/docs/en/dev/build/)

## Як зібрати на Linux 

### Ubuntu 18 (Debian 10)

```sh
sudo apt install -y zip g++ gcc git curl wget nasm yasm libgtk-3-dev clang libxcb-randr0-dev libxdo-dev \
        libxfixes-dev libxcb-shape0-dev libxcb-xfixes0-dev libasound2-dev libpulse-dev cmake make \
        libclang-dev ninja-build libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev
```

### openSUSE Tumbleweed

```sh
sudo zypper install gcc-c++ git curl wget nasm yasm gcc gtk3-devel clang libxcb-devel libXfixes-devel cmake alsa-lib-devel gstreamer-devel gstreamer-plugins-base-devel xdotool-devel
```

### Fedora 28 (CentOS 8)

```sh
sudo yum -y install gcc-c++ git curl wget nasm yasm gcc gtk3-devel clang libxcb-devel libxdo-devel libXfixes-devel pulseaudio-libs-devel cmake alsa-lib-devel gstreamer1-devel gstreamer1-plugins-base-devel
```

### Arch (Manjaro)

```sh
sudo pacman -Syu --needed unzip git cmake gcc curl wget yasm nasm zip make pkg-config clang gtk3 xdotool libxcb libxfixes alsa-lib pipewire
```

### Встановлення vcpkg

```sh
git clone https://github.com/microsoft/vcpkg
cd vcpkg
git checkout 2023.04.15
cd ..
vcpkg/bootstrap-vcpkg.sh
export VCPKG_ROOT=$HOME/vcpkg
vcpkg/vcpkg install libvpx libyuv opus aom
```

### Виправлення libvpx (для Fedora)

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

### Збирання

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env
git clone --branch ultrapro --recurse-submodules https://github.com/atishoo/rustdesk capylink
cd capylink
mkdir -p target/debug
wget https://raw.githubusercontent.com/c-smile/sciter-sdk/master/bin.lnx/x64/libsciter-gtk.so
mv libsciter-gtk.so target/debug
VCPKG_ROOT=$HOME/vcpkg cargo run
```

## Як зібрати за допомогою Docker

Спочатку клонуйте репозиторій і зберіть Docker-контейнер:

```sh
git clone --branch ultrapro https://github.com/atishoo/rustdesk capylink
cd capylink
git submodule update --init --recursive
docker build -t "capylink-builder" .
```

Після цього щоразу, коли потрібно зібрати застосунок, запускайте таку команду:

```sh
docker run --rm -it -v $PWD:/home/user/capylink -v capylink-git-cache:/home/user/.cargo/git -v capylink-registry-cache:/home/user/.cargo/registry -e PUID="$(id -u)" -e PGID="$(id -g)" capylink-builder
```

Перша збірка може тривати довше, доки залежності не буде кешовано. Наступні збірки виконуватимуться швидше. Якщо потрібно передати додаткові аргументи команді збірки, додайте їх після `capylink-builder` у кінці команди `docker run`. Наприклад, для оптимізованої release-збірки додайте `--release`. Отриманий виконуваний файл буде доступний у каталозі `target` вашої системи та його можна запустити так:

```sh
target/debug/capylink
```

Або, якщо використовується release-збірка:

```sh
target/release/capylink
```

Запускайте ці команди з кореневого каталогу репозиторію capylink, інакше застосунок може не знайти потрібні ресурси. Також зверніть увагу, що інші підкоманди cargo, наприклад `install` або `run`, наразі не підтримуються цим способом, оскільки вони встановлюватимуть або запускатимуть програму всередині контейнера, а не на хост-системі.

## Структура файлів

- **[libs/hbb_common](https://github.com/atishoo/rustdesk/tree/ultrapro/libs/hbb_common)**: відеокодек, конфігурація, обгортка tcp/udp та інші допоміжні функції, спільні із сервером
- **[libs/base](https://github.com/atishoo/rustdesk/tree/ultrapro/libs/base)**: protobuf, функції файлової системи для передавання файлів, код клавіатури та платформоспецифічний код, що використовуються лише цим застосунком
- **[libs/scrap](https://github.com/atishoo/rustdesk/tree/ultrapro/libs/scrap)**: захоплення екрана
- **[libs/enigo](https://github.com/atishoo/rustdesk/tree/ultrapro/libs/enigo)**: платформоспецифічне керування клавіатурою та мишею
- **[libs/clipboard](https://github.com/atishoo/rustdesk/tree/ultrapro/libs/clipboard)**: реалізація копіювання та вставлення файлів для Windows, Linux і macOS
- **[src/ui](https://github.com/atishoo/rustdesk/tree/ultrapro/src/ui)**: застарілий інтерфейс Sciter (deprecated)
- **[src/server](https://github.com/atishoo/rustdesk/tree/ultrapro/src/server)**: служби аудіо, буфера обміну, введення та відео, а також мережеві з'єднання
- **[src/client.rs](https://github.com/atishoo/rustdesk/tree/ultrapro/src/client.rs)**: встановлення однорангового з'єднання
- **[src/rendezvous_mediator.rs](https://github.com/atishoo/rustdesk/tree/ultrapro/src/rendezvous_mediator.rs)**: зв'язок із [capylink-server](https://github.com/atishoo/rustdesk-server/tree/ultrapro), очікування прямого віддаленого з'єднання (TCP hole punching) або з'єднання через ретранслятор
- **[src/platform](https://github.com/atishoo/rustdesk/tree/ultrapro/src/platform)**: платформоспецифічний код
- **[flutter](https://github.com/atishoo/rustdesk/tree/ultrapro/flutter)**: код Flutter для настільних і мобільних платформ
