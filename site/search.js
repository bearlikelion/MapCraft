// The header search: loads search-index.js on first use and ranks its blocks against the query.
(() => {
	const box = document.querySelector(".search");
	if (!box) return;
	const input = box.querySelector("input");
	const list = box.querySelector(".search-results");
	const root = document.currentScript?.dataset.root || "";
	let entries = null;
	let shown = [];
	let at = -1;

	const esc = (s) => s.replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);
	const load = () => entries || (entries = new Promise((done) => {
		const s = document.createElement("script");
		s.src = root + "search-index.js";
		s.onload = () => done((window.MAPCRAFT_SEARCH || []).map(([title, section, heading, url, text]) =>
			({ title, section, heading, url, text, hay: [title, heading, text].map((x) => x.toLowerCase()) })));
		s.onerror = () => done([]);
		document.head.append(s);
	}));

	function score(e, terms, phrase) {
		const [title, heading, text] = e.hay;
		let total = 0;
		for (const t of terms) {
			const word = new RegExp("(^|[^a-z0-9])" + t.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"));
			const hit = (word.test(title) ? 8 : 0) + (word.test(heading) ? 6 : 0) + (text.includes(t) ? 1 + Math.min(3, text.split(t).length - 2) : 0);
			if (!hit) return 0;
			total += hit;
		}
		if (heading.includes(phrase) || title.includes(phrase)) total += 10;
		return total + (e.section === "Guide" ? 1 : 0);
	}

	// Whole words that contain a search term, so a hit inside "displacement" marks the whole word.
	function mark(text, terms) {
		const words = new RegExp("[\\p{L}\\p{N}_'-]*(?:" + terms.map((t) => t.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")).join("|") + ")[\\p{L}\\p{N}_'-]*", "giu");
		let out = "";
		let last = 0;
		for (const m of text.matchAll(words)) {
			if (!m[0]) continue;
			out += esc(text.slice(last, m.index)) + "<mark>" + esc(m[0]) + "</mark>";
			last = m.index + m[0].length;
		}
		return out + esc(text.slice(last));
	}

	function snippet(text, terms) {
		const low = text.toLowerCase();
		const first = Math.max(0, Math.min(...terms.map((t) => low.indexOf(t)).filter((i) => i >= 0)));
		const start = first < 60 ? 0 : text.lastIndexOf(" ", first - 40) + 1;
		const piece = text.slice(start, start + 220);
		return (start > 0 ? "\u2026" : "") + mark(piece, terms) + (start + 220 < text.length ? "\u2026" : "");
	}

	function select(i) {
		const items = list.querySelectorAll("a[role=option]");
		items[at]?.removeAttribute("aria-selected");
		at = items.length ? (i + items.length) % items.length : -1;
		items[at]?.setAttribute("aria-selected", "true");
		items[at]?.scrollIntoView({ block: "nearest" });
	}

	async function run() {
		const q = input.value.trim().toLowerCase();
		if (!q) return close();
		const all = await load();
		if (input.value.trim().toLowerCase() !== q) return;
		const terms = q.split(/\s+/);
		const ranked = all.map((e) => [score(e, terms, q), e]).filter(([s]) => s > 0).sort((a, b) => b[0] - a[0]);
		// At most four sections per page, so one long page cannot crowd out the rest.
		const pages = new Map();
		shown = [];
		for (const [, e] of ranked) {
			const page = e.url.split("#")[0];
			const items = pages.get(page) || [];
			if (items.length >= 4) continue;
			items.push(e);
			pages.set(page, items);
			shown.push(e);
			if (shown.length >= 12) break;
		}
		const enter = '<svg class="sr-enter" viewBox="0 0 24 24" aria-hidden="true"><path d="M20 5v7a3 3 0 0 1-3 3H6m4-4-4 4 4 4"/></svg>';
		const item = (e) => `<a role="option" href="${root}${esc(e.url)}"><b>${e.heading ? mark(e.heading, terms) : "Overview"}</b><small>${snippet(e.text, terms)}</small>${enter}</a>`;
		list.innerHTML = shown.length
			? [...pages.values()].map((items) => `<div class="sr-group" role="group" aria-label="${esc(items[0].title)}"><p class="sr-page ${esc(items[0].section.toLowerCase())}">${esc(items[0].title)}<span>${esc(items[0].section)}</span></p>${items.map(item).join("")}</div>`).join("")
				+ '<p class="sr-keys"><span><kbd>\u2191</kbd><kbd>\u2193</kbd> to choose</span><span><kbd>Enter</kbd> to open</span><span><kbd>Esc</kbd> to close</span></p>'
			: `<p class="sr-none">No page mentions \u201c${esc(input.value.trim())}\u201d. Try a shorter word, or browse the <a href="${root}guide/index.html">guide</a>.</p>`;
		list.hidden = false;
		box.classList.add("is-open");
		at = -1;
		if (shown.length) select(0);
	}

	function close() {
		list.hidden = true;
		box.classList.remove("is-open");
		at = -1;
	}

	input.addEventListener("input", run);
	input.addEventListener("focus", () => { load(); if (input.value.trim()) run(); });
	input.addEventListener("keydown", (e) => {
		if (e.key === "ArrowDown" || e.key === "ArrowUp") {
			e.preventDefault();
			select(at + (e.key === "ArrowDown" ? 1 : -1));
		} else if (e.key === "Enter" && at >= 0) {
			e.preventDefault();
			list.querySelectorAll("a[role=option]")[at]?.click();
		} else if (e.key === "Escape") {
			input.value ? (input.value = "", close()) : input.blur();
		}
	});
	document.addEventListener("keydown", (e) => {
		const typing = /^(INPUT|TEXTAREA|SELECT)$/.test(document.activeElement?.tagName) || document.activeElement?.isContentEditable;
		if (!typing && (e.key === "/" || ((e.ctrlKey || e.metaKey) && e.key === "k"))) {
			e.preventDefault();
			input.focus();
			input.select();
		}
	});
	document.addEventListener("pointerdown", (e) => { if (!box.contains(e.target)) close(); });
})();
