// Light/dark toggle in the nav. The choice is kept per browser; with none
// saved the page follows the OS. A pre-paint snippet in <head> applies the
// saved theme before the stylesheet renders, so there is no flash.
const root = document.documentElement;
const toggle = document.getElementById("themeToggle");
const prefersDark = matchMedia("(prefers-color-scheme: dark)");

const currentTheme = () =>
	root.dataset.theme || (prefersDark.matches ? "dark" : "light");

function refreshLabel() {
	const next = currentTheme() === "dark" ? "Light" : "Dark";
	toggle.textContent = next;
	toggle.setAttribute("aria-label", `Switch to ${next.toLowerCase()} theme`);
}

toggle.addEventListener("click", () => {
	const next = currentTheme() === "dark" ? "light" : "dark";
	root.dataset.theme = next;
	try {
		localStorage.setItem("theme", next);
	} catch {
		// Storage blocked: the choice lasts for this page only.
	}
	refreshLabel();
});
prefersDark.addEventListener("change", refreshLabel);
refreshLabel();
