"""Exercise the packaged application and its real Linux desktop file picker."""

import json
import os
from pathlib import Path
import subprocess
import tempfile
import time
import urllib.error
import urllib.request

import pyatspi


def eventually(probe, timeout=25):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        value = probe()
        if value:
            return value
        time.sleep(0.15)
    raise AssertionError("The desktop operation did not finish in time.")


with tempfile.TemporaryDirectory(prefix="frameguard-picker-test-") as temp:
    root = Path(temp)
    video = root / "file-picker-fixture.mp4"
    subprocess.run(
        [
            "ffmpeg", "-v", "error", "-f", "lavfi",
            "-i", "color=white:s=320x200:d=1:r=24",
            "-c:v", "libx264", "-pix_fmt", "yuv420p", "-threads", "1", str(video),
        ],
        check=True,
    )

    environment = {
        **os.environ,
        "TAURI_WEBVIEW_AUTOMATION": "true",
        "XDG_CURRENT_DESKTOP": "XFCE",
    }
    log = (root / "webdriver.log").open("w+")
    process = subprocess.Popen(
        ["WebKitWebDriver", "--port=4444"],
        env=environment, stdout=log, stderr=subprocess.STDOUT,
    )
    session = None

    def request(method, path, body=None):
        payload = None if body is None else json.dumps(body).encode()
        req = urllib.request.Request(
            "http://127.0.0.1:4444" + path,
            data=payload,
            headers={"Content-Type": "application/json"},
            method=method,
        )
        try:
            with urllib.request.urlopen(req, timeout=40) as response:
                return json.load(response)["value"]
        except urllib.error.HTTPError as error:
            raise AssertionError(error.read().decode()) from error

    def running():
        try:
            return request("GET", "/status")
        except urllib.error.URLError:
            return None

    def javascript(script):
        return request("POST", "/session/" + session + "/execute/sync",
                       {"script": script, "args": []})

    def click(selector):
        element = request("POST", "/session/" + session + "/element",
                          {"using": "css selector", "value": selector})
        element_id = element["element-6066-11e4-a52e-4f735466cecf"]
        request("POST", "/session/" + session + "/element/" + element_id + "/click", {})

    def picker_window():
        result = subprocess.run(
            ["xdotool", "search", "--onlyvisible", "--name", "^Open a recording$"],
            capture_output=True, text=True,
        )
        windows = result.stdout.strip().splitlines()
        return windows[-1] if windows else None

    def picker_key(window, *keys):
        subprocess.run(["xdotool", "windowactivate", "--sync", window], check=True)
        subprocess.run(["xdotool", "key", "--clearmodifiers", *keys], check=True)

    def accept_picker():
        pending = [pyatspi.Registry.getDesktop(0)]
        while pending:
            node = pending.pop()
            try:
                if node.name.removeprefix("_") in {"Select", "Open", "Choose"}:
                    return node.queryAction().doAction(0)
                pending.extend(child for child in node if child)
            except Exception:
                # A cancelled chooser may disappear while AT-SPI updates its tree.
                continue
        return False

    try:
        eventually(running)
        created = request(
            "POST", "/session",
            {"capabilities": {"alwaysMatch": {
                "webkitgtk:browserOptions": {"binary": "/usr/bin/frameguard-desktop"},
            }}},
        )
        session = created["sessionId"]
        eventually(lambda: javascript(
            "return document.getElementById('profileView')?.hidden === false"
            " || document.getElementById('dashboard')?.hidden === false;"
        ))
        if javascript("return !document.getElementById('profileView').hidden;"):
            click("#profileSkip")
        eventually(lambda: javascript("return !document.getElementById('dashboard').hidden;"))

        for _ in range(2):
            click("#openBtn")
            window = eventually(picker_window)
            picker_key(window, "Escape")
            eventually(lambda: javascript("return !document.getElementById('openBtn').disabled;"))

        click("#openBtn")
        window = eventually(picker_window)
        time.sleep(0.5)
        picker_key(window, "ctrl+l")
        subprocess.run(
            ["xdotool", "type", "--clearmodifiers", "--delay", "1", str(video)],
            check=True,
        )
        time.sleep(0.5)
        picker_key(window, "Return")
        eventually(lambda: not picker_window() or accept_picker())
        eventually(lambda: javascript(
            "return document.getElementById('fileName').textContent === 'file-picker-fixture.mp4';"
        ))
        eventually(lambda: javascript("return document.getElementById('videoPreview').readyState >= 1;"))
        click("#playVideo")
        eventually(lambda: javascript("return document.getElementById('videoPreview').currentTime > 0;"))
        click("#startScan")
        eventually(lambda: javascript(
            "return !document.getElementById('scanSummary').hidden"
            " && document.getElementById('scanSummary').textContent.includes('Scan complete');"
        ))
        print("Real file picker opened, cancelled twice, reopened, selected an MP4, and scanned it.")
    except BaseException:
        subprocess.run(["import", "-window", "root", "/tmp/frameguard-picker-failure.png"], check=False)
        pending = [pyatspi.Registry.getDesktop(0)]
        while pending:
            node = pending.pop()
            try:
                if node.name:
                    print("Accessible:", node.getRoleName(), repr(node.name))
                pending.extend(child for child in node if child)
            except Exception:
                continue
        if session:
            print(javascript("return document.body.innerText;")[-6000:])
            print(javascript(
                "const video = document.getElementById('videoPreview');"
                "return {src: video.currentSrc, readyState: video.readyState,"
                " networkState: video.networkState, time: video.currentTime,"
                " error: video.error && {code: video.error.code, message: video.error.message}};"
            ))
        subprocess.run(["xdotool", "search", "--onlyvisible", "--name", "."], check=False)
        log.flush()
        log.seek(0)
        print(log.read()[-12000:])
        raise
    finally:
        if session:
            try:
                request("DELETE", "/session/" + session)
            except (AssertionError, urllib.error.URLError):
                pass
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
        log.close()
