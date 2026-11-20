Name:       capylink
Version:    1.5.3
Release:    0
Summary:    RPM package
License:    GPL-3.0
URL:        https://rustdesk.com
Vendor:     rustdesk <info@rustdesk.com>
Requires:   gtk3 libxcb libXfixes alsa-lib libva gstreamer1-plugins-base
Recommends: libayatana-appindicator-gtk3 libxdo
Provides:   libdesktop_drop_plugin.so()(64bit), libdesktop_multi_window_plugin.so()(64bit), libfile_selector_linux_plugin.so()(64bit), libflutter_custom_cursor_plugin.so()(64bit), libflutter_linux_gtk.so()(64bit), libscreen_retriever_plugin.so()(64bit), libtray_manager_plugin.so()(64bit), liburl_launcher_linux_plugin.so()(64bit), libwindow_manager_plugin.so()(64bit), libwindow_size_plugin.so()(64bit), libtexture_rgba_renderer_plugin.so()(64bit)

# https://docs.fedoraproject.org/en-US/packaging-guidelines/Scriptlets/

%description
The best open-source remote desktop client software, written in Rust.

%prep
# we have no source, so nothing here

%build
# we have no source, so nothing here

# %global __python %{__python3}

%install

mkdir -p "%{buildroot}/usr/share/capylink" && cp -r ${HBB}/flutter/build/linux/x64/release/bundle/* -t "%{buildroot}/usr/share/capylink"
mkdir -p "%{buildroot}/usr/bin"
install -Dm 644 $HBB/res/capylink.service -t "%{buildroot}/usr/share/capylink/files"
install -Dm 644 $HBB/res/capylink.desktop -t "%{buildroot}/usr/share/capylink/files"
install -Dm 644 $HBB/res/capylink-link.desktop -t "%{buildroot}/usr/share/capylink/files"
install -Dm 644 $HBB/res/128x128@2x.png "%{buildroot}/usr/share/icons/hicolor/256x256/apps/capylink.png"
install -Dm 644 $HBB/res/scalable.svg "%{buildroot}/usr/share/icons/hicolor/scalable/apps/capylink.svg"

%files
/usr/share/capylink/*
/usr/share/capylink/files/capylink.service
/usr/share/icons/hicolor/256x256/apps/capylink.png
/usr/share/icons/hicolor/scalable/apps/capylink.svg
/usr/share/capylink/files/capylink.desktop
/usr/share/capylink/files/capylink-link.desktop

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
ln -sf /usr/share/capylink/capylink /usr/bin/capylink
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
    rm /usr/bin/capylink || true
    rmdir /usr/lib/capylink || true
    rmdir /usr/local/capylink || true
    rmdir /usr/share/capylink || true
    rm /usr/share/applications/capylink.desktop || true
    rm /usr/share/applications/capylink-link.desktop || true
    update-desktop-database
  ;;
  1)
    # for upgrade
    rmdir /usr/lib/capylink || true
    rmdir /usr/local/capylink || true
  ;;
esac
