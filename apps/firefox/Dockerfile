# Firefox on a virtual X display, viewed through noVNC on :6080. Runs as non-root with no capabilities.
FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends firefox-esr xvfb x11vnc novnc websockify fonts-dejavu-core ca-certificates \
 && rm -rf /var/lib/apt/lists/* \
 && echo '{"policies":{"OverrideFirstRunPage":"","DontCheckDefaultBrowser":true,"DisableTelemetry":true,"DisableAppUpdate":true}}' \
    > /usr/lib/firefox-esr/distribution/policies.json \
 && useradd -m browser
USER browser
ENV DISPLAY=:0
EXPOSE 6080
CMD ["sh", "-c", "Xvfb :0 -screen 0 1280x800x24 -nolisten tcp & until [ -e /tmp/.X11-unix/X0 ]; do sleep 0.1; done; x11vnc -display :0 -forever -shared -nopw -localhost -quiet & websockify --web /usr/share/novnc 6080 localhost:5900 & exec firefox-esr --width 1280 --height 800 ${DOCKBROWSER_URL:+\"$DOCKBROWSER_URL\"}"]
