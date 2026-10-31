# Fedora XFCE desktop, viewed through noVNC on :6080. Runs as non-root with no capabilities.
FROM fedora:latest
RUN dnf install -y xorg-x11-server-Xvfb x11vnc novnc python3-websockify dbus-x11 \
      xfce4-session xfce4-settings xfwm4 xfce4-panel xfdesktop xfce4-terminal thunar firefox dejavu-sans-fonts \
 && dnf clean all \
 && echo '<meta http-equiv="refresh" content="0;url=vnc.html?autoconnect=1&resize=scale">' > /usr/share/novnc/index.html \
 && useradd -m fedora
USER fedora
ENV DISPLAY=:0
EXPOSE 6080
CMD ["sh", "-c", "Xvfb :0 -screen 0 1280x800x24 -nolisten tcp & until [ -e /tmp/.X11-unix/X0 ]; do sleep 0.1; done; x11vnc -display :0 -forever -shared -nopw -localhost -quiet & websockify --web /usr/share/novnc 6080 localhost:5900 & exec dbus-launch xfce4-session"]
