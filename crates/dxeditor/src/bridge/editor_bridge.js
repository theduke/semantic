(() => {
  const root = document.querySelector('[data-dxeditor-id="__EDITOR_ID__"]');
  if (!root || root.__dxeditorBridge) return;

  const chars = value => Array.from(value || '').length;
  const ancestor = (node, attr) => {
    let current = node?.nodeType === Node.ELEMENT_NODE ? node : node?.parentElement;
    while (current && current !== root.parentElement) {
      if (current.hasAttribute?.(attr)) return current;
      current = current.parentElement;
    }
    return null;
  };
  const point = (node, offset) => {
    const block = ancestor(node, 'data-block-id');
    if (!block) return null;
    const range = document.createRange();
    range.setStart(block, 0);
    try { range.setEnd(node, offset); } catch (_) { return null; }
    return { block_id: block.dataset.blockId, inline_id: null, offset: chars(range.toString()) };
  };
  const selection = () => {
    const value = document.getSelection();
    if (!value?.anchorNode || !value?.focusNode) return null;
    const anchor = point(value.anchorNode, value.anchorOffset);
    const focus = point(value.focusNode, value.focusOffset);
    return anchor && focus ? { anchor, focus } : null;
  };
  const emit = value => dioxus.send(value);
  let composing = false;
  let selectionTimer;
  let inputInFlight = false;
  const inputQueue = [];

  const sendInput = (inputType, data) => {
    const current = selection();
    if (!current) {
      inputInFlight = false;
      root.__dxeditorSuppressSelection = false;
      return;
    }
    inputInFlight = true;
    root.__dxeditorSuppressSelection = true;
    emit({ kind: 'beforeInput', input_type: inputType, data, selection: current });
  };

  const acknowledgeInput = () => {
    const next = inputQueue.shift();
    if (next) {
      // setBaseAndExtent has completed, so the next operation is based on the
      // acknowledged model caret rather than a transient browser position.
      queueMicrotask(() => sendInput(next.inputType, next.data));
    } else {
      inputInFlight = false;
      root.__dxeditorSuppressSelection = false;
    }
  };

  const beforeinput = event => {
    if (composing) return;
    event.preventDefault();
    const input = { inputType: event.inputType, data: event.data || '' };
    if (inputInFlight) inputQueue.push(input);
    else sendInput(input.inputType, input.data);
  };
  const selectionchange = () => {
    if (root.__dxeditorSuppressSelection) return;
    clearTimeout(selectionTimer);
    selectionTimer = setTimeout(() => {
      if (root.__dxeditorSuppressSelection) return;
      const current = selection();
      if (current && root.contains(document.getSelection()?.anchorNode))
        emit({ kind: 'selectionChange', selection: current });
    }, 0);
  };
  const compositionstart = () => { composing = true; };
  const compositionend = event => {
    composing = false;
    const block = ancestor(event.target, 'data-block-id');
    const current = selection();
    if (block && current) {
      root.__dxeditorSuppressSelection = true;
      emit({ kind: 'compositionEnd', block_id: block.dataset.blockId,
        text: block.textContent || '', selection: current });
    }
  };
  const paste = event => {
    const current = selection();
    if (!current) return;
    event.preventDefault();
    root.__dxeditorSuppressSelection = true;
    emit({ kind: 'paste', text: event.clipboardData?.getData('text/plain') || '', selection: current });
  };

  root.addEventListener('beforeinput', beforeinput);
  root.addEventListener('compositionstart', compositionstart);
  root.addEventListener('compositionend', compositionend);
  root.addEventListener('paste', paste);
  document.addEventListener('selectionchange', selectionchange);
  root.__dxeditorBridge = {
    beforeinput, compositionstart, compositionend, paste, selectionchange,
    acknowledgeInput
  };
})();
