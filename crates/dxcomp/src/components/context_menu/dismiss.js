const id = await dioxus.recv();
const dismiss = (event) => {
    const menu = document.getElementById(id);
    if (menu && !menu.contains(event.target)) dioxus.send(true);
};
document.addEventListener("pointerdown", dismiss, true);
document.addEventListener("focusin", dismiss, true);
await dioxus.recv();
document.removeEventListener("pointerdown", dismiss, true);
document.removeEventListener("focusin", dismiss, true);
