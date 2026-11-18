(() => {
  'use strict';

  window.createClipPlayer = (dialog, timestamp) => {
    const find = selector => dialog.querySelector(selector);
    const stage = find('#clipStage');
    const video = find('#clipVideo');
    const controls = find('#clipControls');
    const play = find('#clipPlay');
    const mute = find('#clipMute');
    const seek = find('#clipSeek');
    const volume = find('#clipVolume');
    const fullscreen = find('#clipFullscreen');

    function buttonState(button, icon, label) {
      button.title = label;
      button.setAttribute('aria-label', label);
      for (const item of button.querySelectorAll('[data-icon]')) item.hidden = item.dataset.icon !== icon;
    }

    function updateTime() {
      const duration = Number.isFinite(video.duration) ? video.duration : 0;
      const elapsed = duration ? Math.min(video.currentTime, duration) : 0;
      seek.max = String(duration || 1);
      seek.disabled = !duration;
      seek.value = String(elapsed);
      seek.style.setProperty('--fill-pct', (duration ? elapsed / duration * 100 : 0) + '%');
      seek.setAttribute('aria-valuetext', timestamp(elapsed) + ' of ' + timestamp(duration));
      find('#clipClock').textContent = timestamp(elapsed) + ' / ' + timestamp(duration);
    }

    function updatePlay() {
      if (video.ended) buttonState(play, 'replay', 'Replay clip');
      else if (video.paused) buttonState(play, 'play', 'Play');
      else buttonState(play, 'pause', 'Pause');
    }

    function updateVolume() {
      const silent = video.muted || video.volume === 0;
      buttonState(mute, silent ? 'mute' : 'sound', silent ? 'Unmute' : 'Mute');
      volume.value = String(video.volume);
      volume.style.setProperty('--fill-pct', (silent ? 0 : video.volume * 100) + '%');
    }

    function updateFullscreen() {
      const expanded = document.fullscreenElement === stage || dialog.classList.contains('clip-expanded');
      buttonState(fullscreen, expanded ? 'shrink' : 'expand', expanded ? 'Exit fullscreen' : 'Fullscreen');
    }

    function togglePlayback() {
      if (!video.paused && !video.ended) {
        video.pause();
        return;
      }
      if (video.ended) video.currentTime = 0;
      video.play().catch(updatePlay);
    }

    async function toggleFullscreen() {
      if (document.fullscreenElement === stage) {
        await document.exitFullscreen();
      } else if (!dialog.classList.contains('clip-expanded') && document.fullscreenEnabled && stage.requestFullscreen) {
        try { await stage.requestFullscreen(); }
        catch { if (dialog.open) dialog.classList.add('clip-expanded'); }
      } else {
        dialog.classList.toggle('clip-expanded');
      }
      updateFullscreen();
    }

    for (const event of ['loadedmetadata', 'durationchange', 'timeupdate', 'seeking', 'emptied']) {
      video.addEventListener(event, updateTime);
    }
    for (const event of ['play', 'pause', 'ended', 'emptied']) video.addEventListener(event, updatePlay);
    video.addEventListener('volumechange', updateVolume);
    document.addEventListener('fullscreenchange', updateFullscreen);
    play.addEventListener('click', togglePlayback);
    video.addEventListener('click', togglePlayback);
    seek.addEventListener('input', () => { if (!seek.disabled) video.currentTime = seek.valueAsNumber; });
    volume.addEventListener('input', () => {
      video.volume = volume.valueAsNumber;
      video.muted = false;
    });
    mute.addEventListener('click', () => {
      const silent = video.muted || video.volume === 0;
      if (video.volume === 0) video.volume = 0.5;
      video.muted = !silent;
    });
    fullscreen.addEventListener('click', () => { toggleFullscreen().catch(updateFullscreen); });
    stage.addEventListener('keydown', event => {
      if (controls.hidden || event.target.closest('input, button')) return;
      if (event.key === ' ') togglePlayback();
      else if (event.key === 'ArrowLeft' || event.key === 'ArrowRight') {
        if (!seek.disabled) {
          const step = event.key === 'ArrowLeft' ? -1 : 1;
          video.currentTime = Math.min(video.duration, Math.max(0, video.currentTime + step));
        }
      } else return;
      event.preventDefault();
    });

    return {
      open(url) {
        video.src = url;
        video.hidden = false;
        controls.hidden = false;
        updateTime();
        updatePlay();
        updateVolume();
        video.play().catch(updatePlay);
      },
      clear() {
        video.pause();
        video.removeAttribute('src');
        video.load();
        video.hidden = true;
        controls.hidden = true;
        dialog.classList.remove('clip-expanded');
        if (document.fullscreenElement === stage) document.exitFullscreen().catch(updateFullscreen);
        updateFullscreen();
      },
    };
  };
})();
