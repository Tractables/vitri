// A viewer for one text file of any size: only the lines in view are on the
// page, so a formula of any size scrolls at the same speed.

// A line is cut at this many characters on screen; Save writes it whole.
const SHOWN_LINE_CHARS = 2000;
// Browsers cap the height of an element; past this the view maps its scroll
// position onto the lines instead of giving each line its own pixels.
const MAX_SCROLL_HEIGHT = 8e6;
// Pixels per line, as styles.css sets it.
const LINE = 20;

export class TextView {
  #view;
  #spacer;
  #holder;
  #content = "";
  #starts = new Uint32Array(1);
  #lines = 0;
  #queued = false;

  // `view` scrolls; `spacer` inside it takes the file's size, and `holder`
  // carries the lines in view.
  constructor(view, spacer, holder) {
    this.#view = view;
    this.#spacer = spacer;
    this.#holder = holder;
    view.addEventListener("scroll", () => {
      if (!this.#queued) {
        this.#queued = true;
        requestAnimationFrame(() => this.#render());
      }
    });
    new ResizeObserver(() => this.#render()).observe(view);
  }

  // Shows `content` from its first line, and returns its line count.
  show(content) {
    let lines = 0;
    for (let at = content.indexOf("\n"); at !== -1; at = content.indexOf("\n", at + 1)) lines += 1;
    if (content.length > 0 && !content.endsWith("\n")) lines += 1;
    const starts = new Uint32Array(lines + 1);
    let widest = 0;
    let line = 0;
    let begin = 0;
    for (let at = content.indexOf("\n"); at !== -1 && line < lines; at = content.indexOf("\n", at + 1)) {
      starts[line++] = begin;
      widest = Math.max(widest, at - begin);
      begin = at + 1;
    }
    if (line < lines) {
      starts[line++] = begin;
      widest = Math.max(widest, content.length - begin);
    }
    starts[lines] = content.length + 1;
    this.#content = content;
    this.#starts = starts;
    this.#lines = lines;
    this.#spacer.style.height = `${Math.min(lines * LINE, MAX_SCROLL_HEIGHT)}px`;
    this.#spacer.style.width = `calc(${Math.min(widest, SHOWN_LINE_CHARS) + 1}ch + 6.5em)`;
    this.#view.scrollTop = 0;
    this.#view.scrollLeft = 0;
    this.#render();
    return lines;
  }

  #render() {
    this.#queued = false;
    const view = this.#view;
    const inView = Math.ceil(view.clientHeight / LINE) + 1;
    const total = this.#lines * LINE;
    const scaled = total > MAX_SCROLL_HEIGHT;
    const room = Math.max(1, Math.min(total, MAX_SCROLL_HEIGHT) - view.clientHeight);
    const first = scaled
      ? Math.min(Math.max(0, this.#lines - inView + 1), Math.round((view.scrollTop / room) * Math.max(0, this.#lines - inView + 1)))
      : Math.floor(view.scrollTop / LINE);
    this.#holder.style.transform = `translateY(${scaled ? view.scrollTop : first * LINE}px)`;
    const rows = [];
    const make = (name, className, text) => {
      const node = document.createElement(name);
      if (className) node.className = className;
      if (text !== undefined) node.textContent = text;
      return node;
    };
    for (let i = first; i < Math.min(this.#lines, first + inView); i++) {
      const begin = this.#starts[i];
      const end = this.#starts[i + 1] - 1;
      const row = make("div");
      row.append(make("span", "", String(i + 1)));
      if (end - begin > SHOWN_LINE_CHARS) {
        row.append(document.createTextNode(this.#content.slice(begin, begin + SHOWN_LINE_CHARS)));
        row.append(make("em", "", ` … ${(end - begin - SHOWN_LINE_CHARS).toLocaleString("en-US")} more characters; Save has the whole line`));
      } else {
        row.append(document.createTextNode(this.#content.slice(begin, end).replace(/\r$/, "")));
      }
      rows.push(row);
    }
    if (this.#lines === 0) rows.push(make("div", "cut", "(empty file)"));
    this.#holder.replaceChildren(...rows);
  }
}
