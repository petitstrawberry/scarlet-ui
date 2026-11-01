/** Own one lazy specimen without depending on a framework or browser globals. */
export function createSpecimenController({
  load, onLive = () => {}, onFallback = () => {},
  setInteractive = () => {}, onState = () => {},
}) {
  let status = 'fallback';
  let active = false;
  let started = false;
  let terminal = false;
  let app;
  let pending;

  const transition = (next) => {
    if (status !== next) {
      status = next;
      onState(next);
    }
  };
  const stop = () => {
    const current = app;
    app = undefined;
    if (current) {
      try { current.stop(); } catch { /* Cleanup must not revive the specimen. */ }
    }
  };
  const fail = (error) => {
    if (terminal) return;
    terminal = true;
    stop();
    setInteractive(false);
    transition('fallback');
    onFallback(error);
  };
  const sync = () => {
    if (!app || terminal) return;
    if (active) {
      if (status !== 'live') {
        app.resume();
        transition('live');
        setInteractive(true);
        onLive();
      }
    } else if (status !== 'paused') {
      app.pause();
      transition('paused');
      setInteractive(false);
    }
  };
  const begin = () => {
    started = true;
    transition('loading');
    // Start inside the promise chain so synchronous loader failures are caught too.
    pending = Promise.resolve().then(load).then(async (loaded) => {
      app = loaded;
      if (terminal) { stop(); return; }
      await app.start_specimen();
      if (terminal) {
        // dispose may have stopped an in-flight start; stop its resolved runner too.
        try { loaded.stop(); } catch { /* Best-effort cleanup. */ }
        return;
      }
      sync();
    }).catch(fail);
    return pending;
  };

  return {
    get status() { return status; },
    setActive(value) {
      if (terminal) return pending;
      active = Boolean(value);
      if (active && !started) return begin();
      if (status !== 'loading') {
        try { sync(); } catch (error) { fail(error); }
      }
      return pending;
    },
    checkError() {
      if (!app || terminal || status === 'loading') return;
      try {
        const error = app.runtime_error();
        if (error) fail(error);
      } catch (error) { fail(error); }
    },
    dispose() {
      if (status === 'disposed') return;
      terminal = true;
      active = false;
      stop();
      setInteractive(false);
      transition('disposed');
    },
  };
}
