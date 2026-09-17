// vitri runs in a worker, so the page stays live during a run and a run can
// be stopped: cancelling ends the worker and starts another, which loads the
// module again. A run asked for before the worker is ready, or while another
// runs, starts once it is.
//
// The page hears about it through `on`: `loading()` when a worker starts,
// `ready(capabilities)` when its module is up, `started()` when a run begins,
// then `done(result, elapsed)` or `failed(error)` for that run, and
// `notLoaded(message)` when the module does not come up.

export class Runner {
  #url;
  #on;
  #worker = null;
  #ready = false;
  #running = false;
  #queued = null;

  constructor(url, on) {
    this.#url = url;
    this.#on = on;
    this.#start();
  }

  get running() {
    return this.#running;
  }

  // Runs `request` over `dimacs`, ending a run in progress first. The run
  // starts now, or when the module is ready.
  run(dimacs, request) {
    this.#queued = { dimacs, request };
    if (this.#running) {
      this.#running = false;
      this.#restart();
    } else if (this.#ready) {
      this.#send();
    }
  }

  // Ends the run in progress, if there is one, and says whether there was.
  cancel() {
    if (!this.#running) return false;
    this.#running = false;
    this.#queued = null;
    this.#restart();
    return true;
  }

  #start() {
    this.#ready = false;
    this.#on.loading();
    this.#worker = new Worker(this.#url);
    this.#worker.onmessage = (event) => {
      const message = event.data;
      if (message.ready === true) {
        this.#ready = true;
        this.#on.ready(message.capabilities);
        if (this.#queued !== null) this.#send();
      } else if (message.error !== undefined) {
        if (this.#running) {
          this.#running = false;
          this.#on.failed(message.error);
        } else {
          this.#on.notLoaded(message.error.message);
        }
      } else if (message.result !== undefined && this.#running) {
        this.#running = false;
        this.#on.done(message.result, message.elapsed);
      }
    };
    this.#worker.onerror = (event) => {
      event.preventDefault();
      const text = event.message || "the worker stopped";
      if (this.#running) {
        this.#running = false;
        this.#on.failed({ kind: "worker", message: text });
        this.#restart();
      } else {
        this.#on.notLoaded(text);
      }
    };
  }

  #restart() {
    this.#worker.terminate();
    this.#start();
  }

  #send() {
    const job = this.#queued;
    this.#queued = null;
    this.#running = true;
    this.#on.started();
    this.#worker.postMessage(job);
  }
}
