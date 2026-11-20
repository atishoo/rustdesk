Name:       capylink
Version:    1.5.3
Release:    0
Summary:    RPM package
License:    GPL-3.0
URL:        https://rustdesk.com
Vendor:     rustdesk <info@rustdesk.com>
Requires:   gtk3 libxcb libXfixes alsa-lib libva2 gstreamer1-plugins-base
Recommends: libayatana-appindicator-gtk3 libxdo

# https://docs.fedoraproject.org/en-US/packaging-guidelines/Scriptlets/

%description
The best open-source remote desktop client software, written in Rust.

%prep
# we have no source, so nothing here

%build
# we have no source, so nothing here

%global __python %{__python3}

%install
mkdir -p %{buildroot}/usr/bin/
mkdir -p %{buildroot}/usr/share/capylink/
mkdir -p %{buildroot}/usr/share/capylink/files/
mkdir -p %{buildroot}/usr/share/icons/hicolor/256x256/apps/
mkdir -p %{buildroot}/usr/share/icons/hicolor/scalable/apps/
install -m 755 $HBB/target/release/capylink %{buildroot}/usr/bin/capylink
install $HBB/libsciter-gtk.so %{buildroot}/usr/share/capylink/libsciter-gtk.so
install $HBB/res/capylink.service %{buildroot}/usr/share/capylink/files/
install $HBB/res/128x128@2x.png %{buildroot}/usr/share/icons/hicolor/256x256/apps/capylink.png
install $HBB/res/scalable.svg %{buildroot}/usr/share/icons/hicolor/scalable/apps/capylink.svg
install $HBB/res/capylink.desktop %{buildroot}/usr/share/capylink/files/
install $HBB/res/capylink-link.desktop %{buildroot}/usr/share/capylink/files/

%files
/usr/bin/capylink
/usr/share/capylink/libsciter-gtk.so
/usr/share/capylink/files/capylink.service
/usr/share/icons/hicolor/256x256/apps/capylink.png
/usr/share/icons/hicolor/scalable/apps/capylink.svg
/usr/share/capylink/files/capylink.desktop
/usr/share/capylink/files/capylink-link.desktop
/usr/share/capylink/files/__pycache__/*

%changelog
# let's skip this for now

%pre
# can do something for centos7
case "$1" in
  1)
    # for install
  ;;
  2)
    # for upgrade
    systemctl stop capylink || true
  ;;
esac

%post
cp /usr/share/capylink/files/capylink.service /etc/systemd/system/capylink.service
cp /usr/share/capylink/files/capylink.desktop /usr/share/applications/
cp /usr/share/capylink/files/capylink-link.desktop /usr/share/applications/
systemctl daemon-reload
systemctl enable capylink
systemctl start capylink
update-desktop-database

%preun
case "$1" in
  0)
    # for uninstall
    systemctl stop capylink || true
    systemctl disable capylink || true
    rm /etc/systemd/system/capylink.service || true
  ;;
  1)
    # for upgrade
  ;;
esac

%postun
case "$1" in
  0)
    # for uninstall
    rm /usr/share/applications/capylink.desktop || true
    rm /usr/share/applications/capylink-link.desktop || true
    update-desktop-database
  ;;
  1)
    # for upgrade
  ;;
esac
