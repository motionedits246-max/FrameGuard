(() => {
  'use strict';

  const $ = selector => document.querySelector(selector);
  const $$ = selector => [...document.querySelectorAll(selector)];
  const bridge = window.__TAURI__;
  const invoke = (command, arguments_) => bridge?.core.invoke(command, arguments_)
    ?? Promise.reject(new Error('Open FrameGuard Desktop to use this action.'));
  const icon = name => '<i data-lucide="' + name + '" aria-hidden="true"></i>';
  const drawIcons = () => window.lucide?.createIcons();
  const escape = value => String(value).replace(/[&<>"']/g, c => ({
    '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;',
  })[c]);
  const shortTime = seconds => {
    const n = Math.max(0, Math.floor(seconds));
    return String(Math.floor(n / 60)).padStart(2, '0') + ':' + String(n % 60).padStart(2, '0');
  };
  const timestamp = seconds => {
    const tenths = Math.round(seconds * 10);
    return shortTime(tenths / 10) + '.' + (tenths % 10);
  };
  const count = (number, noun) => number.toLocaleString('en-US') + ' ' + noun + (number === 1 ? '' : 's');
  const riskNames = { all: 'All', low: 'Low', medium: 'Medium', high: 'High' };
  const rulesKey = 'frameguard-wordlist-v1';
  const player = $('#videoPreview');
  const state = {
    video: null, fps: 1, rangeMode: 'whole', start: 0, end: 0, phrases: [],
    rules: [], findings: [], selected: null, filter: 'all', reveal: false,
    cliFormat: 'json', exportFormat: 'json', exportMask: true, view: 'scan',
    scanned: false, opening: false,
  };
  const job = { id: null, status: 'idle', done: 0, total: 0, reused: 0, time: 0, findings: 0 };
  let nextRuleId = 1;
  let nextJobId = 1;
  let nextFindingId = 1;
  let editingRuleId = null;
  let testVersion = 0;
  let restored = false;

  try {
    const saved = JSON.parse(localStorage.getItem(rulesKey));
    if (Array.isArray(saved) && saved.every(rule =>
      Number.isSafeInteger(rule?.id) && ['plain', 'regex'].includes(rule.type) &&
      Object.hasOwn(riskNames, rule.risk) && rule.risk !== 'all' &&
      typeof rule.name === 'string' && typeof rule.pattern === 'string'
    )) {
      state.rules = saved;
      nextRuleId = Math.max(0, ...saved.map(rule => rule.id)) + 1;
      restored = true;
    }
  } catch {
    // A session can still edit and export the wordlist.
  }

  function persistRules() {
    try { localStorage.setItem(rulesKey, JSON.stringify(state.rules)); }
    catch { /* A session can still edit and export the wordlist. */ }
  }

  function toast(message, undo) {
    const row = document.createElement('div');
    row.className = 'toast';
    const text = document.createElement('span');
    text.textContent = String(message);
    row.append(text);
    if (undo) {
      const button = document.createElement('button');
      button.type = 'button';
      button.className = 'link';
      button.innerHTML = icon('undo-2') + 'Undo';
      button.addEventListener('click', () => { undo(); row.remove(); });
      row.append(button);
    }
    $('#toasts').append(row);
    drawIcons();
    setTimeout(() => row.remove(), undo ? 7000 : 3800);
  }

  async function copy(text) {
    try {
      await navigator.clipboard.writeText(text);
    } catch {
      const field = document.createElement('textarea');
      field.value = text;
      field.style.cssText = 'position: fixed; opacity: 0';
      (document.querySelector('dialog[open]') ?? document.body).append(field);
      field.select();
      const copied = document.execCommand('copy');
      field.remove();
      if (!copied) { toast('Select the text to copy it.'); return; }
    }
    toast('Copied.');
  }

  function range() {
    if (!state.video) return [0, 0];
    return state.rangeMode === 'whole' ? [0, state.video.duration] : [state.start, state.end];
  }

  const sampleCount = (start, end, fps) => Math.max(0, Math.ceil((end - start) * fps - 1e-9));
  const displayText = (finding, masked) => finding.secret && masked ? '[redacted]' : finding.text;
  const visibleFindings = () => state.findings.filter(finding => state.filter === 'all' || finding.risk === state.filter);
  const getRadio = name => $('input[name="' + name + '"]:checked')?.value;
  const setRadio = (name, value) => $$('input[name="' + name + '"]').forEach(input => { input.checked = input.value === value; });

  function updateTransport() {
    $('#videoSeek').value = String(player.currentTime);
    $('#videoTime').textContent = shortTime(player.currentTime) + ' / ' + shortTime(state.video?.duration ?? 0);
  }

  function renderScan() {
    const video = state.video;
    const [start, end] = range();
    $('#titleFile').textContent = video?.name ?? '';
    $('#fileName').textContent = video?.name ?? 'Choose an MP4';
    $('#sourceTag').hidden = !video;
    $('#statDuration').textContent = video ? shortTime(video.duration) : '—';
    $('#statRes').textContent = video ? video.width + ' × ' + video.height : '—';
    $('#statFps').textContent = video?.fps ? Number(video.fps.toFixed(2)) + ' FPS' : '—';
    $('#openBtn').disabled = state.opening || job.status === 'running';
    $('#playVideo').disabled = !video;
    $('#videoSeek').disabled = !video;
    $('#startScan').disabled = !video || state.opening || job.status === 'running';
    $('#fpsCustom').disabled = job.status === 'running';
    if (document.activeElement !== $('#fpsCustom')) $('#fpsCustom').value = String(state.fps);
    setRadio('fps', String(state.fps));
    setRadio('rangeMode', state.rangeMode);
    $('#rangeFields').hidden = state.rangeMode !== 'range';
    for (const [id, seconds] of [['rStart', state.start], ['rEnd', state.end]]) {
      $('#' + id).max = String(video?.duration ?? 0);
      $('#' + id).value = String(seconds);
      $('#' + id + 'Out').textContent = timestamp(seconds);
    }
    $('#phraseList').innerHTML = state.phrases.map((phrase, index) =>
      '<li class="chip"><span>' + escape(phrase) + '</span><button type="button" data-phrase="' + index +
      '" aria-label="Remove ' + escape(phrase) + '">' + icon('x') + '</button></li>'
    ).join('');
    $('#clearPhrases').hidden = state.phrases.length === 0;
    $('#phraseHint').textContent = state.phrases.length
      ? count(state.phrases.length, 'phrase') + ' for this scan'
      : 'Optional text to flag in this scan.';
    $('#estCount').textContent = sampleCount(start, end, state.fps).toLocaleString('en-US');
    $('#estDetail').textContent = video ? shortTime(end - start) + ' at ' + state.fps + ' FPS · ' +
      count(state.rules.filter(rule => rule.enabled).length, 'rule') : '';
    $('#startLabel').textContent = state.rangeMode === 'whole' ? 'Scan whole video' : 'Scan selected range';
    $('#scanNote').textContent = video ? 'Faster sampling can catch briefer exposures.' : 'Open a recording to start scanning.';
    drawIcons();
  }

  function setCount(number) {
    $('#navCount').hidden = number === 0;
    $('#navCount').textContent = String(number);
  }

  function renderProgress() {
    const pct = job.total ? Math.min(100, Math.round(job.done / job.total * 100)) : 0;
    const busy = job.status === 'running';
    $('#statusPill').classList.toggle('is-busy', busy);
    $('#statusText').textContent = busy ? 'Scanning ' + pct + '%' : 'Local only';
    $('#runPct').textContent = pct + '%';
    $('#progressFill').style.width = pct + '%';
    $('#progress').setAttribute('aria-valuenow', String(pct));
    $('#runStats').textContent = job.done + ' / ' + job.total + ' samples · ' + job.reused +
      ' reused · ' + count(job.findings, 'finding') + ' · ' + timestamp(job.time);
    setCount(busy ? job.findings : state.findings.length);
    if (state.view === 'automate') renderTerminal();
  }

  async function renderCapture(element, finding) {
    const loading = document.createElement('p');
    loading.className = 'frame-note';
    loading.textContent = 'Opening sampled frame';
    element.append(loading);
    try {
      const source = await invoke('inspect_frame', { generation: state.video.generation, seconds: finding.first });
      if (!element.isConnected) return;
      const image = document.createElement('img');
      image.alt = 'Sampled frame at ' + timestamp(finding.first);
      image.addEventListener('load', () => {
        if (!element.isConnected) return;
        element.style.aspectRatio = image.naturalWidth + ' / ' + image.naturalHeight;
        const overlay = document.createElement('span');
        overlay.className = 'capture-hit risk-' + finding.risk;
        overlay.classList.toggle('masked', finding.secret && !state.reveal);
        overlay.setAttribute('aria-hidden', 'true');
        const region = finding.region;
        Object.assign(overlay.style, {
          left: region.x * 100 + '%', top: region.y * 100 + '%',
          width: region.w * 100 + '%', height: region.h * 100 + '%',
        });
        const stamp = document.createElement('span');
        stamp.className = 'frame-time';
        stamp.textContent = timestamp(finding.first);
        element.replaceChildren(image, overlay, stamp);
      }, { once: true });
      image.src = source;
    } catch (error) {
      if (element.isConnected) loading.textContent = String(error);
    }
  }

  function renderFindings() {
    const list = visibleFindings();
    const detail = $('#detail');
    const hasFindings = state.findings.length > 0;
    $('#findingsEmpty').hidden = hasFindings;
    $('#findingsBody').hidden = !hasFindings;
    $('#emptyTitle').textContent = job.status === 'running' ? 'Scanning' : state.scanned ? 'Nothing flagged' : 'No scan yet';
    $('#emptyText').textContent = state.scanned ? 'No rule matched the sampled frames.' : 'Run a scan to inspect findings by timestamp.';
    $('#findingsSub').textContent = hasFindings ? count(state.findings.length, 'finding') + ' in ' + state.video.name : 'Flagged timestamps appear after a scan.';
    $('#revealBtn').disabled = !hasFindings;
    $('#revealBtn').setAttribute('aria-pressed', String(state.reveal));
    $('#revealBtn').innerHTML = icon(state.reveal ? 'eye-off' : 'eye') + (state.reveal ? 'Hide values' : 'Reveal values');
    $('#exportBtn').disabled = !list.length;
    $('#filters').innerHTML = ['all', 'high', 'medium', 'low'].map(risk => {
      const number = state.findings.filter(item => risk === 'all' || risk === item.risk).length;
      return '<button type="button" class="filter risk-' + risk + '" data-risk-filter="' + risk +
        '" aria-pressed="' + (state.filter === risk) + '">' + riskNames[risk] + ' <b>' + number + '</b></button>';
    }).join('');
    if (!list.some(item => item.id === state.selected)) state.selected = list[0]?.id ?? null;
    $('#findingList').innerHTML = list.map(item => {
      const time = item.first === item.last ? timestamp(item.first) : timestamp(item.first) + ' – ' + timestamp(item.last);
      return '<li><button type="button" class="finding risk-' + item.risk + '" data-id="' + item.id +
        '" aria-current="' + (item.id === state.selected) + '"><span class="risk-dot" aria-hidden="true"></span><span class="f-main"><span class="f-name">' +
        escape(item.name) + '</span><code class="f-text">' + escape(displayText(item, !state.reveal)) +
        '</code></span><span class="f-meta"><span class="badge risk-' + item.risk + '">' + riskNames[item.risk] +
        '</span><span class="f-time">' + time + '</span></span></button></li>';
    }).join('') || '<li class="card muted">No findings match this filter.</li>';
    const selected = list.find(item => item.id === state.selected);
    detail.hidden = !selected;
    if (selected) {
      const duration = state.video.duration;
      const markers = list.map(item =>
        '<button type="button" class="marker risk-' + item.risk + '" style="left:' + Math.min(99, item.first / duration * 100) +
        '%;width:' + (item.last - item.first) / duration * 100 + '%" data-id="' + item.id + '" aria-label="' +
        escape(item.name + ' at ' + timestamp(item.first)) + '" aria-current="' + (item.id === selected.id) + '"></button>'
      ).join('');
      detail.innerHTML = '<div class="frame" id="detailFrame"></div><div><div class="timeline" role="group" aria-label="Findings timeline">' +
        markers + '</div><div class="timeline-scale"><span>00:00</span><span>' + shortTime(duration) +
        '</span></div></div><div class="detail-title"><h2>' + escape(selected.name) +
        '</h2><span class="badge risk-' + selected.risk + '">' + riskNames[selected.risk] +
        ' risk</span></div><code class="match">' + escape(displayText(selected, !state.reveal)) +
        '</code><dl class="stats"><div><dt>First seen</dt><dd>' + timestamp(selected.first) +
        '</dd></div><div><dt>Last seen</dt><dd>' + timestamp(selected.last) +
        '</dd></div><div><dt>Samples hit</dt><dd>' + selected.samples_hit +
        '</dd></div></dl><div class="actions"><button type="button" class="btn" id="seekFinding">' + icon('play') +
        'Play in Scan</button><button type="button" class="btn" id="rescanBtn" ' + (job.status === 'running' ? 'disabled' : '') + '>' +
        icon('crosshair') + 'Rescan nearby at 5 FPS</button></div>';
      if (state.view === 'findings') renderCapture($('#detailFrame'), selected);
    }
    drawIcons();
  }

  function mergeFindings(incoming, merge, fps) {
    const merged = merge ? state.findings.slice() : [];
    for (const hit of incoming) {
      const current = merged.find(finding =>
        finding.name === hit.name && finding.risk === hit.risk && finding.text === hit.text &&
        Math.abs(finding.region.x - hit.region.x) < 0.02 && Math.abs(finding.region.y - hit.region.y) < 0.02 &&
        finding.first <= hit.last + 1 / fps && hit.first <= finding.last + 1 / fps
      );
      if (!current) {
        merged.push({ ...hit, id: String(nextFindingId++) });
        continue;
      }
      if (hit.first < current.first) {
        current.first = hit.first;
        current.region = hit.region;
      }
      current.last = Math.max(current.last, hit.last);
      current.sample_times_ms = [...new Set([...current.sample_times_ms, ...hit.sample_times_ms])];
      current.samples_hit = current.sample_times_ms.length;
    }
    state.findings = merged.sort((a, b) => a.first - b.first);
  }

  async function startScan({ start, end, fps, merge }) {
    if (!state.video || state.opening || job.status === 'running') return;
    if (start >= end) { toast('Choose a valid time range.'); return; }
    if (!state.rules.some(rule => rule.enabled) && !state.phrases.length) {
      toast('Enable a rule or add custom text before scanning.');
      return;
    }
    player.pause();
    const id = String(nextJobId++);
    Object.assign(job, { id, status: 'running', done: 0, total: sampleCount(start, end, fps), reused: 0, time: start,
      findings: 0, start, end, fps });
    $('#scanIdle').hidden = true;
    $('#scanSummary').hidden = true;
    $('#scanRun').hidden = false;
    $('#cancelScan').disabled = false;
    $('#runLabel').textContent = merge ? 'Rescanning at ' + fps + ' FPS' : 'Scanning recording';
    renderScan();
    renderProgress();
    let message;
    try {
      const report = await invoke('scan_recording', {
        generation: state.video.generation, scanId: id,
        options: { start, end, fps, rules: state.rules, phrases: state.phrases },
      });
      state.scanned = true;
      if (!merge) state.selected = null;
      mergeFindings(report.findings, merge, fps);
      Object.assign(job, report.progress, { status: report.status });
      message = (report.status === 'cancelled' ? 'Scan cancelled. ' : 'Scan complete. ') +
        count(state.findings.length, 'finding') + ' · ' + report.progress.reused + ' OCR results reused.';
    } catch (error) {
      job.status = 'failed';
      message = String(error);
    } finally {
      $('#scanIdle').hidden = false;
      $('#scanRun').hidden = true;
      $('#scanSummary').hidden = false;
      $('#scanSummary').innerHTML = icon(job.status === 'failed' ? 'triangle-alert' : 'circle-check') +
        '<span class="text"><strong>' + escape(message) + '</strong></span>' +
        (state.findings.length ? '<button class="btn" type="button" data-goto="findings">' + icon('arrow-right') + 'View findings</button>' : '');
      renderAll();
      if (state.view !== 'scan') toast(message);
    }
  }

  async function cancel() {
    if (job.status !== 'running') return;
    const id = job.id;
    $('#cancelScan').disabled = true;
    $('#runLabel').textContent = 'Cancelling scan';
    try {
      const accepted = await invoke('cancel_scan', { scanId: id });
      if (!accepted && job.id === id && job.status === 'running') {
        setTimeout(() => { if (job.id === id) cancel(); }, 80);
      }
    } catch (error) {
      $('#cancelScan').disabled = false;
      toast(error);
    }
  }

  function renderRules() {
    $('#rulesSub').textContent = state.rules.filter(rule => rule.enabled).length + ' of ' +
      count(state.rules.length, 'rule') + ' enabled.';
    $('#ruleList').innerHTML = state.rules.map(rule => {
      const menu = ['low', 'medium', 'high'].map(risk =>
        '<option value="' + risk + '"' + (risk === rule.risk ? ' selected' : '') + '>' + riskNames[risk] + '</option>'
      ).join('');
      const controls = [['test', 'flask-conical'], ['edit', 'pencil'], ['duplicate', 'copy'], ['delete', 'trash-2']].map(([action, glyph]) =>
        '<button type="button" class="icon-btn' + (action === 'delete' ? ' is-danger' : '') +
        '" data-rule-action="' + action + '" aria-label="' + action + ' ' + escape(rule.name) + '">' + icon(glyph) + '</button>'
      ).join('');
      return '<li class="rule card' + (rule.enabled ? '' : ' is-off') + '" data-rule-id="' + rule.id +
        '"><label class="switch"><input type="checkbox" data-rule-action="toggle" aria-label="Enable ' + escape(rule.name) +
        '"' + (rule.enabled ? ' checked' : '') + '><span></span></label><div class="rule-main"><div class="rule-name">' +
        escape(rule.name) + '<span class="tag">' + (rule.type === 'regex' ? 'Regex' : 'Plain text') +
        '</span>' + (rule.secret ? '<span class="tag">Masked</span>' : '') + '</div><code>' +
        escape(rule.pattern) + '</code></div><div class="rule-side"><select class="risk-select risk-' + rule.risk +
        '" data-rule-action="risk" aria-label="Risk for ' + escape(rule.name) + '">' + menu + '</select>' + controls + '</div></li>';
    }).join('') || '<li class="card muted">Add a rule to begin a wordlist.</li>';
    drawIcons();
  }

  function draftRule() {
    return {
      id: editingRuleId ?? nextRuleId, name: $('#rName').value.trim(),
      type: getRadio('rType'), pattern: $('#rPattern').value,
      risk: getRadio('rRisk'), secret: $('#rSecret').checked, enabled: true,
    };
  }

  function editRule(rule, testOnly = false) {
    editingRuleId = rule?.id ?? null;
    $('#ruleDialogTitle').textContent = testOnly ? 'Test ' + rule.name : rule ? 'Edit rule' : 'Add rule';
    $('#rName').value = rule?.name ?? '';
    $('#rPattern').value = rule?.pattern ?? '';
    $('#rSecret').checked = rule?.secret ?? false;
    setRadio('rType', rule?.type ?? 'plain');
    setRadio('rRisk', rule?.risk ?? 'medium');
    $('#ruleDialog').showModal();
    (testOnly ? $('#rSample') : $('#rName')).focus();
    testRule();
  }

  async function testRule() {
    const version = ++testVersion;
    const rule = draftRule();
    const sample = $('#rSample').value;
    const hasPattern = Boolean(rule.pattern.trim());
    $('#rSave').disabled = true;
    let matches = [];
    let error;
    if (hasPattern) {
      try { matches = await invoke('test_wordlist_rule', { rule, sample }); }
      catch (failure) { error = String(failure); }
    }
    if (version !== testVersion) return false;
    let markup = '';
    let end = 0;
    for (const [start, stop] of matches) {
      markup += escape(sample.slice(end, start)) + '<mark>' + escape(sample.slice(start, stop)) + '</mark>';
      end = stop;
    }
    $('#rPreview').innerHTML = markup + escape(sample.slice(end));
    $('#rError').textContent = error ?? '';
    $('#rPattern').setAttribute('aria-invalid', String(Boolean(error)));
    $('#rCount').textContent = error ? 'Fix the pattern to continue.' : hasPattern ? count(matches.length, 'match') : 'Enter a pattern to test.';
    $('#rSave').disabled = Boolean(error) || !hasPattern;
    return !error && hasPattern;
  }

  function commandText() {
    const quote = value => "'" + value.replaceAll("'", "''") + "'";
    const parts = ['frameguard scan ' + quote(state.video?.path ?? 'recording.mp4'), '--fps ' + state.fps];
    if (state.rangeMode === 'range') parts.push('--start ' + state.start, '--end ' + state.end);
    parts.push('--rules wordlist.json');
    for (const phrase of state.phrases) parts.push('--text ' + quote(phrase));
    parts.push('--format ' + state.cliFormat, '--output findings.' + state.cliFormat);
    return parts.join(' ' + String.fromCharCode(96) + '\n  ');
  }

  function renderTerminal() {
    $('#cliCmd').textContent = commandText();
    $('#tuiFile').textContent = state.video?.name ?? 'Choose an MP4';
    $('#tuiPreviewTitle').textContent = 'At ' + timestamp(player.currentTime);
    const preview = $('#tuiFrame');
    const canvas = document.createElement('canvas');
    if (player.readyState >= 2 && player.videoWidth) {
      canvas.width = 192;
      canvas.height = Math.max(1, Math.round(canvas.width * player.videoHeight / player.videoWidth));
      canvas.getContext('2d').drawImage(player, 0, 0, canvas.width, canvas.height);
      preview.replaceChildren(canvas);
    } else {
      const note = document.createElement('p');
      note.className = 'frame-note';
      note.textContent = 'Open a recording to preview a frame.';
      preview.replaceChildren(note);
    }
    $('#tuiStatus').textContent = [
      'status   ' + job.status, 'rate     ' + (job.status === 'running' ? job.fps : state.fps) + ' FPS',
      'samples  ' + job.done + ' / ' + job.total,
      'reused   ' + job.reused + ' OCR results',
      'findings ' + (job.status === 'running' ? job.findings : state.findings.length),
    ].join('\n');
  }

  function reportText() {
    const rows = visibleFindings().map(finding => ({
      rule: finding.name, source: finding.custom ? 'custom_text' : 'wordlist', risk: finding.risk,
      first_seconds: finding.first, last_seconds: finding.last, samples_hit: finding.samples_hit,
      text: displayText(finding, state.exportMask), masked: finding.secret && state.exportMask,
      region: finding.region,
    }));
    if (state.exportFormat === 'json') return JSON.stringify({
      video: state.video.name, scan_status: job.status, findings: rows,
    }, null, 2);
    const cell = value => {
      let text = String(value);
      if (typeof value === 'string' && /^[=+\-@]/.test(text.trimStart())) text = "'" + text;
      return /[",\r\n]/.test(text) ? '"' + text.replaceAll('"', '""') + '"' : text;
    };
    return ['rule,source,risk,first_seconds,last_seconds,samples_hit,text,masked,region_x,region_y,region_w,region_h',
      ...rows.map(row => [
        row.rule, row.source, row.risk, row.first_seconds, row.last_seconds, row.samples_hit,
        row.text, row.masked, row.region.x, row.region.y, row.region.w, row.region.h,
      ].map(cell).join(','))].join('\n');
  }

  function renderExport() {
    setRadio('exFormat', state.exportFormat);
    $('#exMask').checked = state.exportMask;
    $('#exPreview').textContent = reportText();
    $('#exNote').textContent = count(visibleFindings().length, 'finding');
  }

  async function save(filename, contents) {
    try { if (await invoke('save_export', { filename, contents })) toast('Saved ' + filename + '.'); }
    catch (error) { toast(error); }
  }

  function go(view) {
    state.view = view;
    for (const tab of $$('.tab')) {
      const active = tab.dataset.view === view;
      tab.setAttribute('aria-selected', String(active));
      tab.tabIndex = active ? 0 : -1;
    }
    for (const section of $$('.view')) section.hidden = section.id !== 'view-' + view;
    if (view === 'findings') renderFindings();
    if (view === 'automate') renderTerminal();
    if (view === 'scan') renderScan();
  }

  function renderAll() {
    renderScan();
    renderFindings();
    renderRules();
    renderProgress();
    if (state.view === 'automate') renderTerminal();
  }

  $('#openBtn').addEventListener('click', async () => {
    if (state.opening || job.status === 'running') return;
    state.opening = true;
    renderScan();
    try {
      const video = await invoke('open_recording');
      if (!video) return;
      player.pause();
      Object.assign(state, { video, rangeMode: 'whole', start: 0, end: Math.min(30, video.duration),
        findings: [], selected: null, filter: 'all', reveal: false, scanned: false });
      Object.assign(job, { status: 'idle', done: 0, total: 0, reused: 0, time: 0, findings: 0 });
      $('#scanSummary').hidden = true;
      $('#videoPlaceholder').hidden = true;
      $('#videoSeek').max = String(video.duration);
      player.hidden = false;
      player.src = bridge.core.convertFileSrc(video.path);
      updateTransport();
    } catch (error) { toast(error); }
    finally { state.opening = false; renderAll(); }
  });

  $('#playVideo').addEventListener('click', () => {
    if (player.paused) {
      if (player.ended) player.currentTime = 0;
      player.play().catch(() => toast('Windows could not play this video track.'));
    } else player.pause();
  });
  for (const event of ['play', 'pause', 'ended', 'emptied']) player.addEventListener(event, () => {
    const paused = player.paused || player.ended;
    $('#playVideo').innerHTML = icon(paused ? 'play' : 'pause');
    $('#playVideo').setAttribute('aria-label', paused ? 'Play video' : 'Pause video');
    drawIcons();
  });
  for (const event of ['loadeddata', 'timeupdate', 'seeked']) player.addEventListener(event, () => {
    updateTransport();
    if (state.view === 'automate') renderTerminal();
  });
  player.addEventListener('error', () => toast('Video playback is unavailable for this track. FFmpeg can still scan it.'));
  $('#videoSeek').addEventListener('input', event => {
    if (player.readyState) player.currentTime = Number(event.target.value);
  });

  $$('input[name="fps"]').forEach(input => input.addEventListener('change', () => {
    state.fps = Number(input.value);
    renderScan();
  }));
  $('#fpsCustom').addEventListener('input', event => {
    const value = event.target.valueAsNumber;
    const valid = Number.isFinite(value) && value >= 0.1 && value <= 30;
    event.target.setAttribute('aria-invalid', String(!valid));
    if (valid) { state.fps = value; renderScan(); }
  });
  $('#fpsCustom').addEventListener('blur', event => {
    event.target.value = String(state.fps);
    event.target.setAttribute('aria-invalid', 'false');
  });
  $$('input[name="rangeMode"]').forEach(input => input.addEventListener('change', () => { state.rangeMode = input.value; renderScan(); }));
  $('#rStart').addEventListener('input', event => {
    state.start = Math.max(0, Math.min(event.target.valueAsNumber, state.end - 0.01));
    renderScan();
  });
  $('#rEnd').addEventListener('input', event => {
    state.end = Math.min(state.video?.duration ?? 0, Math.max(event.target.valueAsNumber, state.start + 0.01));
    renderScan();
  });

  $('#phraseForm').addEventListener('submit', event => {
    event.preventDefault();
    const field = $('#phraseInput');
    const phrase = field.value.trim();
    if (phrase && !state.phrases.some(item => item.toLowerCase() === phrase.toLowerCase())) state.phrases.push(phrase);
    field.value = '';
    field.focus();
    renderScan();
  });
  $('#phraseList').addEventListener('click', event => {
    const button = event.target.closest('[data-phrase]');
    if (!button) return;
    const index = Number(button.dataset.phrase);
    const [removed] = state.phrases.splice(index, 1);
    renderScan();
    $('#phraseInput').focus();
    toast('Custom text removed.', () => {
      if (!state.phrases.some(item => item.toLowerCase() === removed.toLowerCase())) state.phrases.splice(index, 0, removed);
      renderScan();
    });
  });
  $('#clearPhrases').addEventListener('click', () => {
    const removed = state.phrases;
    state.phrases = [];
    renderScan();
    toast('Custom text cleared.', () => {
      const restored = new Map([...state.phrases, ...removed].map(item => [item.toLowerCase(), item]));
      state.phrases = [...restored.values()];
      renderScan();
    });
  });

  $('#startScan').addEventListener('click', () => {
    const [start, end] = range();
    startScan({ start, end, fps: state.fps, merge: false });
  });
  $('#cancelScan').addEventListener('click', cancel);
  $('#filters').addEventListener('click', event => {
    const button = event.target.closest('[data-risk-filter]');
    if (button) { state.filter = button.dataset.riskFilter; renderFindings(); }
  });
  $('#findingList').addEventListener('click', event => {
    const button = event.target.closest('[data-id]');
    if (button) { state.selected = button.dataset.id; renderFindings(); }
  });
  $('#detail').addEventListener('click', event => {
    const marker = event.target.closest('.marker');
    if (marker) { state.selected = marker.dataset.id; renderFindings(); return; }
    const finding = state.findings.find(item => item.id === state.selected);
    if (!finding) return;
    if (event.target.closest('#seekFinding')) {
      go('scan');
      if (player.readyState) player.currentTime = finding.first;
      player.play().catch(() => toast('This video track could not be played.'));
    }
    if (event.target.closest('#rescanBtn')) {
      const start = Math.max(0, finding.first - 5);
      const end = Math.min(state.video.duration, finding.first + 5);
      go('scan');
      startScan({ start, end, fps: 5, merge: true });
    }
  });
  $('#revealBtn').addEventListener('click', () => { state.reveal = !state.reveal; renderFindings(); });
  $('#exportBtn').addEventListener('click', () => { renderExport(); $('#exportDialog').showModal(); });
  $$('input[name="exFormat"]').forEach(input => input.addEventListener('change', () => {
    state.exportFormat = input.value;
    renderExport();
  }));
  $('#exMask').addEventListener('change', event => { state.exportMask = event.target.checked; renderExport(); });
  $('#exCopy').addEventListener('click', () => copy(reportText()));
  $('#exDownload').addEventListener('click', () => save('frameguard-findings.' + state.exportFormat, reportText()));

  $('#addRule').addEventListener('click', () => editRule());
  $('#saveWordlist').addEventListener('click', () => save('wordlist.json', JSON.stringify(state.rules, null, 2)));
  $('#ruleList').addEventListener('change', event => {
    const row = event.target.closest('[data-rule-id]');
    const rule = state.rules.find(item => item.id === Number(row?.dataset.ruleId));
    if (!rule) return;
    if (event.target.dataset.ruleAction === 'toggle') rule.enabled = event.target.checked;
    if (event.target.dataset.ruleAction === 'risk') rule.risk = event.target.value;
    persistRules();
    renderRules();
  });
  $('#ruleList').addEventListener('click', event => {
    const button = event.target.closest('button[data-rule-action]');
    const row = button?.closest('[data-rule-id]');
    const index = state.rules.findIndex(item => item.id === Number(row?.dataset.ruleId));
    if (index < 0) return;
    const rule = state.rules[index];
    switch (button.dataset.ruleAction) {
      case 'edit': editRule(rule); return;
      case 'test': editRule(rule, true); return;
      case 'duplicate': state.rules.splice(index + 1, 0, { ...rule, id: nextRuleId++, name: rule.name + ' copy' }); break;
      case 'delete':
        state.rules.splice(index, 1);
        toast('Rule removed.', () => {
          state.rules.splice(Math.min(index, state.rules.length), 0, rule);
          persistRules();
          renderRules();
        });
        break;
    }
    persistRules();
    renderRules();
  });
  ['#rPattern', '#rSample'].forEach(selector => $(selector).addEventListener('input', testRule));
  $$('input[name="rType"]').forEach(input => input.addEventListener('change', testRule));
  $('#ruleForm').addEventListener('submit', async event => {
    event.preventDefault();
    const draft = draftRule();
    if (!draft.name || !await testRule() || !$('#ruleDialog').open) return;
    const rule = state.rules.find(item => item.id === draft.id);
    if (rule) Object.assign(rule, { ...draft, enabled: rule.enabled });
    else state.rules.push({ ...draft, id: nextRuleId++ });
    $('#ruleDialog').close();
    persistRules();
    renderRules();
    toast('Rule saved.');
  });

  $$('input[name="cliFormat"]').forEach(input => input.addEventListener('change', () => { state.cliFormat = input.value; renderTerminal(); }));
  $('#copyCli').addEventListener('click', () => copy(commandText()));
  $('#tabs').addEventListener('click', event => {
    const tab = event.target.closest('.tab');
    if (tab) go(tab.dataset.view);
  });
  $('#tabs').addEventListener('keydown', event => {
    const tabs = $$('.tab');
    const index = tabs.indexOf(document.activeElement);
    const step = { ArrowRight: 1, ArrowDown: 1, ArrowLeft: -1, ArrowUp: -1 }[event.key];
    if (index < 0 || !step) return;
    event.preventDefault();
    const next = tabs[(index + step + tabs.length) % tabs.length];
    next.focus();
    go(next.dataset.view);
  });
  document.addEventListener('click', event => {
    const link = event.target.closest('[data-goto]');
    if (link) go(link.dataset.goto);
    const close = event.target.closest('[data-close]');
    if (close) close.closest('dialog').close();
  });

  async function initialize() {
    renderAll();
    if (!bridge) {
      $('#scanNote').textContent = 'Open in FrameGuard Desktop to scan recordings.';
      return;
    }
    try {
      await bridge.event.listen('scan-progress', ({ payload }) => {
        if (job.status !== 'running' || payload.scan_id !== job.id) return;
        Object.assign(job, payload.progress);
        renderProgress();
      });
      if (!restored) {
        state.rules = await invoke('default_wordlist');
        nextRuleId = Math.max(0, ...state.rules.map(rule => rule.id)) + 1;
        renderAll();
      }
    } catch (error) { toast(error); }
  }

  initialize();
})();
