// The Made with MapCraft carousel: one map per slide, a filmstrip to jump, and a full size viewer.
(() => {
	const show = document.querySelector(".showcase");
	if (!show) return;
	const track = show.querySelector(".slides");
	const slides = [...track.children];
	const strip = show.querySelector(".filmstrip");
	const thumbs = [...strip.querySelectorAll("button")];
	const count = document.querySelector(".made-count b");
	const viewer = document.querySelector(".viewer");
	const motion = () => (matchMedia("(prefers-reduced-motion: reduce)").matches ? "auto" : "smooth");
	let at = 0;
	// A programmatic scroll passes other slides on its way; their positions are not a new choice.
	let pending = -1;
	let pendingTimer = 0;

	const go = (i) => {
		at = (i + slides.length) % slides.length;
		pending = at;
		clearTimeout(pendingTimer);
		pendingTimer = setTimeout(() => (pending = -1), 1200);
		track.scrollTo({ left: at * track.clientWidth, behavior: motion() });
		sync();
	};

	function sync() {
		count.textContent = at + 1;
		thumbs.forEach((t, i) => (i === at ? t.setAttribute("aria-current", "true") : t.removeAttribute("aria-current")));
		const t = thumbs[at];
		if (t) strip.scrollTo({ left: t.offsetLeft - (strip.clientWidth - t.offsetWidth) / 2, behavior: motion() });
		slides.forEach((s, i) => (s.inert = i !== at));
		if (viewer.open) fill();
	}

	function fill() {
		const s = slides[at];
		const [lit, top] = s.querySelectorAll("img");
		const [vLit, vTop] = viewer.querySelectorAll("img");
		viewer.querySelector("h2 code").textContent = s.dataset.name;
		viewer.querySelector(".viewer-meta").textContent = `${s.dataset.meta}, ${at + 1} of ${slides.length}`;
		for (const [from, to] of [[lit, vLit], [top, vTop]]) {
			to.src = from.currentSrc || from.src;
			to.alt = from.alt;
		}
	}

	let settle = 0;
	track.addEventListener("scroll", () => {
		cancelAnimationFrame(settle);
		settle = requestAnimationFrame(() => {
			const i = Math.round(track.scrollLeft / track.clientWidth);
			if (pending >= 0) {
				if (i === pending) pending = -1;
				return;
			}
			if (i !== at && i >= 0 && i < slides.length) {
				at = i;
				sync();
			}
		});
	});

	const keys = (e) => {
		if (e.key === "ArrowRight" || e.key === "ArrowLeft") {
			e.preventDefault();
			go(at + (e.key === "ArrowRight" ? 1 : -1));
		}
	};
	track.addEventListener("keydown", keys);
	viewer.addEventListener("keydown", keys);
	document.querySelectorAll(".step.prev").forEach((b) => b.addEventListener("click", () => go(at - 1)));
	document.querySelectorAll(".step.next").forEach((b) => b.addEventListener("click", () => go(at + 1)));
	thumbs.forEach((t, i) => t.addEventListener("click", () => go(i)));
	show.querySelectorAll(".open").forEach((b) => b.addEventListener("click", () => {
		fill();
		viewer.showModal();
	}));
	viewer.querySelector(".close").addEventListener("click", () => viewer.close());
	viewer.addEventListener("click", (e) => { if (e.target === viewer) viewer.close(); });
	addEventListener("resize", () => track.scrollTo({ left: at * track.clientWidth }));
	sync();
})();
