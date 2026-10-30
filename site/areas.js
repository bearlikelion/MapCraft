// Pictures with numbered areas: hovering, focusing or tapping a pin, or hovering an item of the numbered
// list under the picture, outlines that area and shows the item's text on the picture.
for (const fig of document.querySelectorAll(".has-areas")) {
	const frame = fig.querySelector(".areas-frame");
	const items = [...(fig.closest("section")?.querySelectorAll(".block-text > ol > li") || [])];
	const areas = [...frame.querySelectorAll(".area")];
	const pins = [...frame.querySelectorAll(".pin")];
	const tip = document.createElement("div");
	tip.className = "area-tip";
	tip.hidden = true;
	frame.append(tip);

	const show = (n) => {
		frame.classList.add("is-on");
		for (const a of areas) a.classList.toggle("on", a.dataset.n === n);
		items.forEach((li, i) => li.classList.toggle("on", String(i + 1) === n));
		for (const p of pins) p.setAttribute("aria-expanded", String(p.dataset.n === n));
		const pin = pins.find((p) => p.dataset.n === n);
		const li = items[Number(n) - 1];
		if (!pin || !li) return (tip.hidden = true);
		tip.innerHTML = li.innerHTML;
		tip.hidden = false;
		const room = 8;
		let x = pin.offsetLeft - tip.offsetWidth / 2;
		x = Math.max(room, Math.min(x, frame.clientWidth - tip.offsetWidth - room));
		let y = pin.offsetTop + pin.offsetHeight / 2 + room;
		if (y + tip.offsetHeight > frame.clientHeight - room) y = pin.offsetTop - pin.offsetHeight / 2 - room - tip.offsetHeight;
		tip.style.left = x + "px";
		tip.style.top = Math.max(room, y) + "px";
	};
	const hide = () => {
		frame.classList.remove("is-on");
		for (const a of areas) a.classList.remove("on");
		for (const li of items) li.classList.remove("on");
		for (const p of pins) p.setAttribute("aria-expanded", "false");
		tip.hidden = true;
	};

	for (const p of pins) {
		p.setAttribute("aria-expanded", "false");
		p.addEventListener("mouseenter", () => show(p.dataset.n));
		p.addEventListener("focus", () => show(p.dataset.n));
		p.addEventListener("click", () => show(p.dataset.n));
	}
	frame.addEventListener("mouseleave", hide);
	frame.addEventListener("focusout", (e) => { if (!frame.contains(e.relatedTarget)) hide(); });
	frame.addEventListener("keydown", (e) => { if (e.key === "Escape") hide(); });
	items.forEach((li, i) => {
		li.addEventListener("mouseenter", () => show(String(i + 1)));
		li.addEventListener("mouseleave", hide);
	});
}
