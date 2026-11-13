/* StyleSmart frontend — every feature calls the real JSON API. */
const api = async (path, opts = {}) => {
  const r = await fetch(path, { headers: { "Content-Type": "application/json" }, ...opts });
  const data = await r.json().catch(() => ({}));
  if (!r.ok) throw new Error(data.error || `Request failed (${r.status})`);
  return data;
};
const esc = s => String(s ?? "").replace(/[&<>"']/g, c => ({"&":"&amp;","<":"&lt;",">":"&gt;",'"':"&quot;","'":"&#39;"}[c]));
const toast = msg => {
  const t = document.createElement("div");
  t.className = "toast"; t.textContent = msg;
  document.body.appendChild(t);
  setTimeout(() => t.remove(), 3000);
};
const swatch = (hex, color) => `<span class="swatch" title="${esc(color)}" style="background:${esc(hex || "#888")}"></span>`;

/* ---- tabs ---- */
document.querySelectorAll("#tabs button").forEach(b => {
  b.onclick = () => {
    document.querySelectorAll("#tabs button").forEach(x => x.classList.remove("active"));
    document.querySelectorAll("section.tab").forEach(x => x.classList.remove("active"));
    b.classList.add("active");
    document.getElementById("tab-" + b.dataset.tab).classList.add("active");
    ({ dashboard: loadDashboard, wardrobe: loadWardrobe, analytics: loadAnalytics,
       fit: loadFit, grooming: loadGrooming, travel: loadPackings }[b.dataset.tab] || (() => {}))();
  };
});
const showTab = name => document.querySelector(`#tabs button[data-tab="${name}"]`).click();

/* ---- dashboard ---- */
async function loadDashboard() {
  const [score, analytics] = await Promise.all([api("/api/score"), api("/api/analytics")]);
  document.getElementById("score-num").textContent = score.score;
  const b = score.breakdown;
  document.getElementById("score-breakdown").innerHTML = Object.entries(b).map(([k, v]) => `
    <div style="margin-bottom:8px">
      <div style="display:flex;justify-content:space-between;font-size:13px">
        <span>${esc(k.replace(/_/g, " "))}</span><span>${v.points} / ${v.max}</span>
      </div>
      <div class="bar"><div style="width:${Math.round(v.points / v.max * 100)}%"></div></div>
    </div>`).join("");
  document.getElementById("glance").innerHTML = `
    <div><div class="stat">${analytics.total_items}</div><div class="stat-label">items owned</div></div>
    <div><div class="stat">$${analytics.total_value.toFixed(0)}</div><div class="stat-label">wardrobe value</div></div>
    <div><div class="stat">${analytics.total_wears}</div><div class="stat-label">total wears</div></div>
    <div><div class="stat">${analytics.average_cost_per_wear !== null ? "$" + analytics.average_cost_per_wear : "–"}</div><div class="stat-label">avg cost / wear</div></div>`;
  document.getElementById("header-stats").innerHTML =
    `<span class="badge gold">Appearance Score ${score.score}/100</span>`;
  const hist = await api("/api/history");
  document.getElementById("recent-outfits").innerHTML = hist.length
    ? hist.map(h => `<div class="check-row"><span class="badge">${esc(h.date)}</span>
        <span>${esc(h.occasion || "outfit")} — ${h.item_ids.length} items</span></div>`).join("")
    : "<p class='sub'>No outfits logged yet.</p>";
}

async function quickOutfit() {
  const o = await api("/api/outfit?occasion=work&temp=26&condition=sunny");
  renderOutfit(o, document.getElementById("quick-outfit"));
}

/* ---- wardrobe ---- */
let editingId = null;
async function loadWardrobe() {
  const cat = document.getElementById("f-category").value;
  const color = document.getElementById("f-color").value.trim();
  const search = document.getElementById("f-search").value.trim();
  const q = new URLSearchParams();
  if (cat) q.set("category", cat);
  if (color) q.set("color", color);
  if (search) q.set("search", search);
  const items = await api("/api/items?" + q.toString());
  document.getElementById("wardrobe-grid").innerHTML = items.map(i => `
    <div class="item-card">
      <div class="top-row">${swatch(i.color_hex, i.color)}
        <div><div class="name">${esc(i.name)}</div>
        <div class="meta">${esc(i.category)} · ${esc(i.color)} · ${esc(i.brand || "—")}</div></div>
      </div>
      <div class="meta">$${i.price} · size ${esc(i.size || "—")} · worn ${i.wears}×
        ${i.wears ? `· $${(i.price / i.wears).toFixed(2)}/wear` : "· never worn"}</div>
      <div class="meta">${esc((i.occasions || []).join(", "))}${i.fit_notes ? " · " + esc(i.fit_notes) : ""}</div>
      <div class="actions">
        <button class="small" onclick="logWear(${i.id})">Log wear</button>
        <button class="small ghost" onclick="editItem(${i.id})">Edit</button>
        <button class="small danger" onclick="deleteItem(${i.id})">Delete</button>
      </div>
    </div>`).join("") || "<p class='sub'>No items match.</p>";
}
async function addItem() {
  const body = {
    name: document.getElementById("a-name").value.trim(),
    category: document.getElementById("a-category").value,
    color: document.getElementById("a-color").value.trim() || "grey",
    brand: document.getElementById("a-brand").value.trim(),
    price: parseFloat(document.getElementById("a-price").value || 0),
    size: document.getElementById("a-size").value.trim(),
    purchase_date: document.getElementById("a-date").value,
    material: document.getElementById("a-material").value.trim(),
    occasions: document.getElementById("a-occasions").value.split(",").map(s => s.trim()).filter(Boolean),
    fit_notes: document.getElementById("a-fit").value.trim(),
  };
  if (!body.name) return toast("Name is required");
  await api("/api/items", { method: "POST", body: JSON.stringify(body) });
  toast("Item added"); loadWardrobe(); loadDashboard();
}
async function logWear(id) {
  await api(`/api/items/${id}/wear`, { method: "POST" });
  toast("Wear logged"); loadWardrobe(); loadDashboard();
}
async function deleteItem(id) {
  if (!confirm("Delete this item?")) return;
  await api(`/api/items/${id}`, { method: "DELETE" });
  toast("Item deleted"); loadWardrobe(); loadDashboard();
}
async function editItem(id) {
  const it = await api(`/api/items/${id}`);
  const wears = prompt(`Wears for "${it.name}" (currently ${it.wears}):`, it.wears);
  if (wears === null) return;
  const price = prompt(`Price $ (currently ${it.price}):`, it.price);
  if (price === null) return;
  await api(`/api/items/${id}`, { method: "PUT",
    body: JSON.stringify({ wears: parseInt(wears, 10), price: parseFloat(price) }) });
  toast("Item updated"); loadWardrobe();
}

/* ---- outfit ---- */
let currentOutfit = null;
async function generateOutfit() {
  const occ = document.getElementById("o-occasion").value;
  const temp = document.getElementById("o-temp").value;
  const cond = document.getElementById("o-condition").value;
  currentOutfit = await api(`/api/outfit?occasion=${occ}&temp=${temp}&condition=${cond}`);
  renderOutfit(currentOutfit, document.getElementById("outfit-result"), true);
}
function renderOutfit(o, el, withLog = false) {
  const slotLabel = { top: "Top", bottom: "Bottom", footwear: "Shoes", outerwear: "Outerwear", accessory: "Accessory", fragrance: "Fragrance" };
  el.innerHTML = `
    <div class="card">
      <h3>${esc(o.occasion)} outfit · ${o.temp}°C · ${esc(o.condition)}</h3>
      ${o.repeated_yesterday ? `<p><span class="badge red">Warning: identical to yesterday's outfit</span></p>` : ""}
      ${Object.entries(o.picks).map(([slot, it]) => `
        <div class="check-row">${swatch(it.color_hex, it.color)}
          <div><b>${slotLabel[slot] || slot}:</b> ${esc(it.name)}
            <span class="badge">${esc(it.brand || "")} · $${it.price}</span>
            ${(o.reasons[slot] || []).map(r => `<div class="reason">${esc(r)}</div>`).join("")}
          </div>
        </div>`).join("")}
      ${o.notes.map(n => `<p class="reason">${esc(n)}</p>`).join("")}
      ${withLog ? `<div style="margin-top:12px"><button onclick="logOutfit()">I wore this outfit</button></div>` : ""}
    </div>`;
}
async function logOutfit() {
  if (!currentOutfit) return;
  const ids = Object.values(currentOutfit.picks).map(i => i.id);
  await api("/api/outfit/log", { method: "POST", body: JSON.stringify({
    item_ids: ids, occasion: currentOutfit.occasion,
    temp: currentOutfit.temp, condition: currentOutfit.condition }) });
  toast("Outfit logged — wears updated"); loadDashboard();
}

/* ---- analytics ---- */
async function loadAnalytics() {
  const a = await api("/api/analytics");
  document.getElementById("analytics-stats").innerHTML = `
    <div class="card"><div class="stat">$${a.total_value.toFixed(0)}</div><div class="stat-label">total wardrobe value</div></div>
    <div class="card"><div class="stat">${a.total_wears}</div><div class="stat-label">total wears</div></div>
    <div class="card"><div class="stat">${a.average_cost_per_wear !== null ? "$" + a.average_cost_per_wear : "–"}</div><div class="stat-label">average cost per wear</div></div>`;
  document.getElementById("roi-table").innerHTML = `
    <table><tr><th>Item</th><th>Price</th><th>Wears</th><th>Cost / wear</th><th>Verdict</th></tr>
    ${a.roi_ranking.map(r => `<tr><td>${esc(r.name)}</td><td>$${r.price}</td><td>${r.wears}</td>
      <td>$${r.cost_per_wear.toFixed(2)}</td><td>${esc(r.verdict)}</td></tr>`).join("")}
    ${a.items.filter(i => i.cost_per_wear === null).map(r => `<tr><td>${esc(r.name)}</td><td>$${r.price}</td><td>0</td><td>–</td><td>${esc(r.verdict)}</td></tr>`).join("")}
    </table>`;
  const list = arr => arr.map(i => `<div class="check-row"><span>${esc(i.name)}</span><span class="badge">${i.wears}×</span></div>`).join("") || "<p class='sub'>—</p>";
  document.getElementById("most-worn").innerHTML = list(a.most_worn);
  document.getElementById("least-worn").innerHTML = list(a.least_worn);
}

/* ---- fit ---- */
async function loadFit() {
  const p = await api("/api/fit-profile");
  document.getElementById("p-top").value = p.sizes.top || "";
  document.getElementById("p-bottom").value = p.sizes.bottom || "";
  document.getElementById("p-footwear").value = p.sizes.footwear || "";
  document.getElementById("p-outerwear").value = p.sizes.outerwear || "";
  document.getElementById("p-pref").value = p.fit_preference || "regular";
  document.getElementById("p-notes").value = p.body_notes || "";
  const items = await api("/api/items");
  const rows = [];
  for (const it of items) {
    if (it.category === "grooming" || it.category === "fragrance") continue;
    const f = await api("/api/fit-score", { method: "POST", body: JSON.stringify({ item_id: it.id }) });
    rows.push({ it, f });
  }
  rows.sort((a, b) => a.f.score - b.f.score);
  document.getElementById("fit-table").innerHTML = `
    <table><tr><th>Item</th><th>Size</th><th>Fit score</th><th>Why</th></tr>
    ${rows.map(({ it, f }) => `<tr><td>${esc(it.name)}</td><td>${esc(it.size)}</td>
      <td><b>${f.score}</b>/100</td><td>${f.reasons.map(esc).join("<br>")}</td></tr>`).join("")}</table>`;
}
async function saveProfile() {
  await api("/api/fit-profile", { method: "PUT", body: JSON.stringify({
    sizes: { top: document.getElementById("p-top").value,
             bottom: document.getElementById("p-bottom").value,
             footwear: document.getElementById("p-footwear").value,
             outerwear: document.getElementById("p-outerwear").value },
    fit_preference: document.getElementById("p-pref").value,
    body_notes: document.getElementById("p-notes").value }) });
  toast("Profile saved"); loadFit();
}
async function checkRisk() {
  const r = await api("/api/return-risk", { method: "POST", body: JSON.stringify({
    category: document.getElementById("r-category").value,
    size: document.getElementById("r-size").value,
    brand: document.getElementById("r-brand").value,
    color: document.getElementById("r-color").value,
    price: parseFloat(document.getElementById("r-price").value || 0) }) });
  const cls = r.band === "Low" ? "green" : r.band === "High" ? "red" : "amber";
  document.getElementById("risk-result").innerHTML = `
    <p><span class="badge ${cls}">${r.band} risk — ${r.risk}/100</span></p>
    ${r.reasons.map(x => `<div class="reason">${esc(x)}</div>`).join("")}`;
}

/* ---- grooming ---- */
async function loadGrooming() {
  const g = await api("/api/grooming");
  document.getElementById("routine-list").innerHTML = g.products.map(p => `
    <label class="check-row ${p.done_today ? "done" : ""}">
      <input type="checkbox" ${p.done_today ? "checked" : ""} onchange="toggleCheck(${p.id}, this.checked)">
      <span class="cname">${esc(p.name)}</span>
      <span class="badge">${esc(p.routine)}</span>
    </label>`).join("");
  document.getElementById("grooming-list").innerHTML = g.products.map(p => {
    const badge = p.expiry_status === "expired" ? `<span class="badge red">EXPIRED</span>`
      : p.expiry_status === "expiring_soon" ? `<span class="badge amber">Expires soon</span>`
      : `<span class="badge green">OK</span>`;
    return `<div class="check-row"><div><b>${esc(p.name)}</b> <span class="badge">${esc(p.category)}</span>
      <div class="meta">Expires ${esc(p.expiry_date || "—")} ${badge}</div>
      <div class="meta">${esc(p.notes || "")}</div></div>
      <button class="small danger" style="margin-left:auto" onclick="deleteGrooming(${p.id})">Delete</button></div>`;
  }).join("");
}
async function toggleCheck(id, done) {
  await api("/api/grooming/check", { method: "POST", body: JSON.stringify({ product_id: id, done }) });
  loadGrooming(); loadDashboard();
}
async function addGrooming() {
  const name = document.getElementById("g-name").value.trim();
  if (!name) return toast("Name is required");
  await api("/api/grooming", { method: "POST", body: JSON.stringify({
    name, category: document.getElementById("g-cat").value,
    routine: document.getElementById("g-routine").value,
    expiry_date: document.getElementById("g-expiry").value }) });
  toast("Product added"); loadGrooming();
}
async function deleteGrooming(id) {
  if (!confirm("Delete this product?")) return;
  await api(`/api/grooming/${id}`, { method: "DELETE" });
  toast("Product deleted"); loadGrooming();
}

/* ---- travel ---- */
async function generatePacking() {
  const dest = document.getElementById("t-dest").value.trim();
  if (!dest) return toast("Destination is required");
  const occ = [...document.getElementById("t-occ").selectedOptions].map(o => o.value);
  const p = await api("/api/packing", { method: "POST", body: JSON.stringify({
    destination: dest, days: parseInt(document.getElementById("t-days").value, 10),
    weather: document.getElementById("t-weather").value, occasions: occ }) });
  document.getElementById("packing-result").innerHTML = `
    <div class="card"><h3>Packing list — ${esc(dest)} (${p.days} days)</h3>
      <p class="sub">${p.total_items} items, all from your wardrobe.</p>
      ${p.items.map((it, i) => `<label class="check-row"><input type="checkbox">
        <span class="cname">${esc(it.name)}</span><span class="badge">${esc(it.category)}</span>
        <span class="meta">${esc(it.reason)}</span></label>`).join("")}
      <h3 style="margin-top:14px">Essentials checklist</h3>
      ${p.essentials_checklist.map(e => `<label class="check-row"><input type="checkbox"><span class="cname">${esc(e)}</span></label>`).join("")}
      <p class="sub" style="margin-top:10px">${esc(p.note)}</p>
    </div>`;
  loadPackings();
}
async function loadPackings() {
  const list = await api("/api/packings");
  document.getElementById("saved-packings").innerHTML = list.length
    ? list.map(p => `<div class="check-row"><span class="badge">${esc(p.destination)}</span>
        <span>${p.days} days · ${p.items.length} items · ${esc(p.created_at)}</span></div>`).join("")
    : "<p class='sub'>No packing lists saved yet.</p>";
}

/* ---- shopping ---- */
async function checkShopping() {
  const r = await api("/api/shopping-check", { method: "POST", body: JSON.stringify({
    category: document.getElementById("s-category").value,
    color: document.getElementById("s-color").value.trim(),
    price: parseFloat(document.getElementById("s-price").value || 0) }) });
  const cls = r.recommended === "buy" ? "buy" : r.recommended === "skip" ? "skip" : "think";
  document.getElementById("shopping-result").innerHTML = `
    <div class="card">
      <div class="verdict ${cls}">${r.verdict}</div>
      ${r.reasoning.map(x => `<div class="reason">${esc(x)}</div>`).join("")}
      ${r.similar.length ? `<h3 style="margin-top:12px">Similar items you own</h3>
        ${r.similar.map(s => `<div class="check-row"><span>${esc(s.name)}</span>
          <span class="badge">${esc(s.color)} · ${s.wears}× · ${s.cost_per_wear !== null ? "$" + s.cost_per_wear + "/wear" : "never worn"}</span></div>`).join("")}` : ""}
    </div>`;
}

/* ---- boot ---- */
loadDashboard();
loadWardrobe();
