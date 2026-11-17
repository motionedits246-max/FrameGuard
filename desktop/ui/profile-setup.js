(() => {
  'use strict';

  window.frameguardProfileSetup = invoke => new Promise(resolve => {
    const view = document.querySelector('#profileView');
    const find = selector => view.querySelector(selector);
    const terms = find('#profileTerms');
    const next = find('#profileNext');
    const back = find('#profileBack');
    const panels = [...view.querySelectorAll('[data-profile-panel]')];
    const custom = [...view.querySelectorAll('.profile-custom input')];
    let step = 0;
    let pending = 0;
    let loading = false;
    let finished = false;

    const role = () => find('[name="profileRole"]:checked')?.value;
    const topics = () => [...view.querySelectorAll('[name="profileTopic"]:checked')].map(input => input.value);
    const selectedTerms = () => [
      ...[...terms.querySelectorAll('input:checked')].map(input => ({ text: input.value, personal: false })),
      ...custom.map(input => input.value.trim()).filter(Boolean).map(text => ({ text, personal: true })),
    ];

    function updateButton() {
      next.disabled = [!role(), !topics().length, loading || !selectedTerms().length][step];
    }

    function showStep(index) {
      step = index;
      find('#profileStep').textContent = 'Step ' + (index + 1) + ' of 3';
      for (const [position, panel] of panels.entries()) panel.hidden = position !== index;
      back.hidden = index === 0;
      next.querySelector('span').textContent = ['Continue', 'Find keywords', 'Use these keywords'][index];
      panels[index].querySelector('h1').focus();
      updateButton();
    }

    function finish(entries) {
      finished = true;
      view.hidden = true;
      resolve(entries);
    }

    function renderWords(words) {
      terms.replaceChildren();
      for (const word of words) {
        const label = document.createElement('label');
        label.className = 'check profile-term';
        const checkbox = document.createElement('input');
        checkbox.type = 'checkbox';
        checkbox.checked = true;
        checkbox.value = word;
        label.append(checkbox, document.createTextNode(word));
        terms.append(label);
      }
      terms.hidden = words.length === 0;
    }

    async function loadKeywords() {
      const request = ++pending;
      loading = true;
      const note = find('#profileNote');
      note.textContent = 'Looking up keywords for your work.';
      find('#profileProgress').hidden = false;
      find('#profileResults').hidden = true;
      showStep(2);
      let words = [];
      let message = 'Add a few words you want FrameGuard to flag.';
      try {
        if (role() !== 'other') words = await invoke('profile_keywords', { role: role(), topics: topics() });
        if (words.length) message = 'Choose which suggestions to add to your wordlist.';
      } catch {
        message = 'Online suggestions are unavailable. Add a few words to scan for.';
      }
      if (finished || pending !== request) return;
      renderWords(words);
      const editor = role() === 'editor' || role() === 'designer';
      const examples = editor
        ? ['Client name', 'private-logo.png', 'Unreleased project']
        : ['Client or project name', 'A private phrase', 'A private filename'];
      custom.forEach((input, index) => { input.placeholder = examples[index]; });
      note.textContent = message;
      find('#profileProgress').hidden = true;
      find('#profileResults').hidden = false;
      loading = false;
      updateButton();
    }

    view.addEventListener('change', updateButton);
    view.addEventListener('input', updateButton);
    find('#profileSkip').addEventListener('click', () => finish([]));
    back.addEventListener('click', () => {
      ++pending;
      loading = false;
      showStep(step - 1);
    });
    next.addEventListener('click', () => {
      if (step === 0) showStep(1);
      else if (step === 1) loadKeywords();
      else finish(selectedTerms());
    });
    view.hidden = false;
    showStep(0);
  });
})();
