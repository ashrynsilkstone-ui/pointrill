%global debug_package %{nil}
# Preserve the portable executable shared with the other packages.
%global __strip /usr/bin/true

Name:           pointrill
Version:        %{pointrill_version}
Release:        1
Summary:        Desktop settings for OpenRazer mice and receivers
License:        NOASSERTION
URL:            https://github.com/ashrynsilkstone-ui/pointrill
Source0:        pointrill-payload.tar
ExclusiveArch:  x86_64
Requires:       libglvnd-glx, libglvnd-egl
Requires:       libwayland-client, libwayland-cursor, libwayland-egl
Requires:       libxkbcommon, libX11, libXcursor, libXi, libXrandr
Suggests:       openrazer-daemon

%description
Pointrill configures OpenRazer mice and receivers. It includes a desktop tray
with battery status, DPI stage and profile switching.

%install
mkdir -p "%{buildroot}"
tar -xf "%{SOURCE0}" -C "%{buildroot}"

%files
%{_bindir}/pointrill
%{_datadir}/applications/io.github.ashrynsilkstone_ui.Pointrill.desktop
%config(noreplace) %{_sysconfdir}/xdg/autostart/io.github.ashrynsilkstone_ui.Pointrill.Tray.desktop
%dir %{_docdir}/pointrill
%doc %{_docdir}/pointrill/*
