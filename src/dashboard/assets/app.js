// aw Native Rust Dashboard Client (Zero External Dependencies)

let allMemories = [];
let recentQueries = [];
let reuseCandidates = [];
let prewriteSearches = [];
let totalCalls = 0;
let totalErrors = 0;
let latestSummaryData = null;

let selectedMemoryIndex = 0;
let selectedReuseIndex = 0;
let activeReuseDetector = 'ALL';

// Theme Toggle
function initTheme() {
  const saved = localStorage.getItem('aw-theme') || 'light';
  document.documentElement.setAttribute('data-theme', saved);
  updateThemeButtonText(saved);
}

function toggleTheme() {
  const current = document.documentElement.getAttribute('data-theme') || 'light';
  const next = current === 'light' ? 'dark' : 'light';
  document.documentElement.setAttribute('data-theme', next);
  localStorage.setItem('aw-theme', next);
  updateThemeButtonText(next);
}

function updateThemeButtonText(theme) {
  const btn = document.getElementById('theme-toggle-btn');
  if (btn) btn.textContent = 'Theme: ' + (theme === 'dark' ? 'Dark' : 'Light');
}

// Tab Switching
function switchTab(tab) {
  const tabs = ['telemetry', 'reuse', 'memory'];
  tabs.forEach(t => {
    const btn = document.getElementById('tab-' + t + '-btn');
    const content = document.getElementById('tab-' + t + '-content');
    if (t === tab) {
      if (btn) btn.classList.add('active');
      if (content) content.classList.remove('hidden');
    } else {
      if (btn) btn.classList.remove('active');
      if (content) content.classList.add('hidden');
    }
  });

  if (tab === 'reuse') {
    populateReuseRepoFilter();
    renderReuseList();
    renderPrewriteTable();
  } else if (tab === 'memory') {
    populateMemoryStoreFilter();
    renderMemoryList();
  }
}

// Data Fetching
function loadSummaryData() {
  fetch('/api/summary')
    .then(r => r.json())
    .then(data => {
      latestSummaryData = data;
      totalCalls = data.total_calls || 0;
      totalErrors = data.error_count || 0;

      const totalCallsEl = document.getElementById('metric-total-calls');
      if (totalCallsEl) totalCallsEl.textContent = totalCalls.toLocaleString();

      const relEl = document.getElementById('metric-reliability');
      if (relEl) {
        relEl.textContent = (data.reliability_pct || 100.0).toFixed(1) + '% clean (' + totalErrors + ' errs)';
      }

      const p50El = document.getElementById('metric-p50');
      if (p50El) p50El.textContent = (data.p50_ms || 0.0).toFixed(1) + 'ms';

      const latSubEl = document.getElementById('metric-latency-sub');
      if (latSubEl) {
        latSubEl.textContent = 'p95 = ' + (data.p95_ms || 0.0).toFixed(1) + 'ms · avg = ' + (data.avg_ms || 0.0).toFixed(1) + 'ms';
      }

      const tokEl = document.getElementById('metric-tokens');
      if (tokEl) tokEl.textContent = '~' + (data.saved_tokens || 0).toLocaleString();

      const dolEl = document.getElementById('metric-dollars');
      if (dolEl) dolEl.textContent = '~$' + (data.saved_dollars || 0.0).toFixed(2) + ' avoided vs full reads';

      const activeReposEl = document.getElementById('metric-active-repos');
      if (activeReposEl) {
        const repoCount = Object.keys(data.repo_aggs || {}).length;
        activeReposEl.textContent = repoCount.toLocaleString();
      }

      const activeWorktreesEl = document.getElementById('metric-active-worktrees');
      if (activeWorktreesEl) {
        const wtCount = Object.values(data.repo_aggs || {}).filter(r => r.worktree).length;
        activeWorktreesEl.textContent = wtCount > 0
          ? wtCount + ' worktree' + (wtCount > 1 ? 's' : '') + ' active'
          : 'Multi-repository attribution';
      }

      const reuseHitsEl = document.getElementById('metric-reuse-hits');
      if (reuseHitsEl) reuseHitsEl.textContent = (data.reuse_hits || 0).toLocaleString();

      const activeMemEl = document.getElementById('metric-active-memories');
      if (activeMemEl) activeMemEl.textContent = data.active_memories || 0;

      // Render Repositories Table
      renderReposTable(data.repo_aggs || {});
      // Render Agents Table
      renderAgentsTable(data.agent_aggs || {}, totalCalls);
      // Render Tools Table
      renderToolsTable(data.tool_aggs || {}, totalCalls);
      // Render Recent Queries
      renderRecentQueries(data.recent_queries || []);
      // Render Errors Triage
      renderErrorsTriage(data.recent_errors || []);
      // Update Memory KPIs
      updateMemoryKpis();
    })
    .catch(() => {});
}

function loadReuseData() {
  fetch('/api/reuse')
    .then(r => r.json())
    .then(data => {
      reuseCandidates = data.candidates || [];
      prewriteSearches = data.prewrite_searches || [];

      const countBadge = document.getElementById('nav-reuse-count');
      if (countBadge) countBadge.textContent = reuseCandidates.length;

      const kpiCount = document.getElementById('reuse-kpi-count');
      if (kpiCount) kpiCount.textContent = reuseCandidates.length;

      const summary = data.summary || {};
      const kpiLoc = document.getElementById('reuse-kpi-loc');
      if (kpiLoc) kpiLoc.textContent = '~' + (summary.preventable_loc || 0).toLocaleString();

      const kpiPre = document.getElementById('reuse-kpi-prewrite');
      if (kpiPre) kpiPre.textContent = summary.prewrite_count || prewriteSearches.length;

      const kpiCallers = document.getElementById('reuse-kpi-callers');
      if (kpiCallers) kpiCallers.textContent = (summary.avg_callers || 0.0).toFixed(1);

      populateReuseRepoFilter();
      renderReuseList();
      renderPrewriteTable();
    })
    .catch(() => {});
}

function loadMemoriesData() {
  fetch('/api/memories')
    .then(r => r.json())
    .then(data => {
      allMemories = data || [];
      updateMemoryKpis();
      populateMemoryStoreFilter();
      renderMemoryList();
    })
    .catch(() => {});
}

function updateMemoryKpis() {
  const currentNotes = allMemories.filter(m => m.is_current);

  const navCount = document.getElementById('nav-mem-count');
  if (navCount) navCount.textContent = currentNotes.length;

  // KPI 1: Read:Write Ratio
  let reads = 0;
  let writes = 0;
  if (latestSummaryData && latestSummaryData.tool_aggs) {
    const aggs = latestSummaryData.tool_aggs;
    reads = (aggs['mem_query'] ? aggs['mem_query'].calls : 0) + (aggs['mem_get'] ? aggs['mem_get'].calls : 0);
    writes = (aggs['mem_write'] ? aggs['mem_write'].calls : 0) + (aggs['mem_touch'] ? aggs['mem_touch'].calls : 0);
  }
  const rwRatio = writes === 0 ? (reads > 0 ? reads.toFixed(1) + '×' : '0.0×') : (reads / writes).toFixed(1) + '×';
  const rwEl = document.getElementById('mem-kpi-rw');
  if (rwEl) rwEl.textContent = rwRatio;
  const rwSubEl = document.getElementById('mem-kpi-rw-sub');
  if (rwSubEl) rwSubEl.textContent = reads + ' reads · ' + writes + ' writes';

  // KPI 2: Session Recall Rate
  const recallPct = latestSummaryData ? (latestSummaryData.session_recall_pct || 0.0).toFixed(1) + '%' : '0.0%';
  const recallEl = document.getElementById('mem-kpi-recall-pct');
  if (recallEl) recallEl.textContent = recallPct;
  const recallSubEl = document.getElementById('mem-kpi-recall-pct-sub');
  if (recallSubEl) {
    const memSessions = latestSummaryData ? (latestSummaryData.memory_session_count || 0) : 0;
    const totalSessions = latestSummaryData ? (latestSummaryData.session_count || 0) : 0;
    recallSubEl.textContent = memSessions + ' of ' + totalSessions + ' sessions';
  }

  // KPI 3: Anchor Currency Rate
  const notesWithRefs = currentNotes.filter(m => m.refs && m.refs.length > 0);
  let freshCount = 0;
  let modifiedCount = 0;
  let brokenCount = 0;
  notesWithRefs.forEach(m => {
    if (m.ref_status === 'broken') brokenCount++;
    else if (m.ref_status === 'modified') modifiedCount++;
    else freshCount++;
  });
  const currencyRate = notesWithRefs.length > 0 ? (100.0 * freshCount / notesWithRefs.length).toFixed(1) + '%' : '100.0%';
  const currencyEl = document.getElementById('mem-kpi-currency');
  if (currencyEl) currencyEl.textContent = currencyRate;
  const currencySubEl = document.getElementById('mem-kpi-currency-sub');
  if (currencySubEl) {
    currencySubEl.textContent = freshCount + ' fresh · ' + modifiedCount + ' modified · ' + brokenCount + ' broken';
  }

  // KPI 4: Guarded Symbols & Hotspots
  const gotchas = currentNotes.filter(m => m.kind === 'gotcha').length;
  const decisions = currentNotes.filter(m => m.kind === 'decision').length;
  const guardedEl = document.getElementById('mem-kpi-guarded');
  if (guardedEl) guardedEl.textContent = gotchas + decisions;
  const guardedSub = document.getElementById('mem-kpi-guarded-sub');
  if (guardedSub) guardedSub.textContent = gotchas + ' gotchas · ' + decisions + ' decisions';

  updateActionCenter(currentNotes, modifiedCount, brokenCount);
}

function updateActionCenter(currentNotes, modifiedCount, brokenCount) {
  const centerEl = document.getElementById('mem-action-center');
  const summaryEl = document.getElementById('action-center-summary');
  const itemsEl = document.getElementById('action-center-items');
  if (!centerEl || !itemsEl) return;

  const actionable = currentNotes.filter(m => m.ref_status === 'modified' || m.ref_status === 'broken');
  if (actionable.length === 0) {
    centerEl.classList.add('hidden');
    itemsEl.innerHTML = '';
    return;
  }

  centerEl.classList.remove('hidden');
  if (summaryEl) {
    summaryEl.textContent = modifiedCount + ' modified · ' + brokenCount + ' broken requiring attention';
  }

  itemsEl.innerHTML = '';
  actionable.forEach(m => {
    const isModified = m.ref_status === 'modified';
    const badge = isModified
      ? '<span class="badge status-warn">[MODIFIED REF]</span>'
      : '<span class="badge status-err">[BROKEN REF]</span>';
    const explanation = isModified
      ? 'Target file was modified after note creation. Review note and touch to confirm currency.'
      : 'Referenced symbol or file path is missing from current code index (.aw/index.db).';
    const touchBtn = isModified
      ? '<button class="btn-touch" onclick="touchMemory(\'' + escapeHtml(m.key) + '\', ' + (m.store === 'global') + ')">Touch Anchor</button>'
      : '';

    const refsList = (m.refs || []).map(r => '<code>' + escapeHtml(r) + '</code>').join(', ');

    const item = document.createElement('div');
    item.className = 'action-item';
    item.innerHTML =
      '<div class="action-item-info">' +
      '  <div class="action-item-title">' +
      '    ' + badge +
      '    <strong>' + escapeHtml(m.key) + '</strong>' +
      '    <span class="badge badge-repo">' + escapeHtml(m.store) + '</span>' +
      '    <span class="text-muted" style="font-size:0.75rem;">r' + m.rev + '</span>' +
      '  </div>' +
      '  <div class="action-item-desc text-muted">' + explanation + '</div>' +
      '  <div class="action-item-refs text-muted" style="font-size:0.75rem;">Refs: ' + refsList + '</div>' +
      '</div>' +
      touchBtn;

    itemsEl.appendChild(item);
  });
}

function touchMemory(key, isGlobal) {
  fetch('/api/memory/touch', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ key: key, global: !!isGlobal })
  })
    .then(r => r.json())
    .then(res => {
      if (res.ok) {
        loadMemoriesData();
      } else {
        alert('Failed to touch memory note: ' + (res.error || 'unknown error'));
      }
    })
    .catch(err => {
      alert('Network error touching memory note: ' + err.message);
    });
}

// Telemetry Views Rendering
function renderReposTable(repoAggs) {
  const tbody = document.getElementById('repos-tbody');
  const select = document.getElementById('repo-filter');
  if (!tbody) return;

  const entries = Object.entries(repoAggs);
  if (entries.length === 0) {
    tbody.innerHTML = '<tr><td colspan="4" class="empty-cell">No repositories recorded</td></tr>';
    return;
  }

  entries.sort((a, b) => b[1].calls - a[1].calls);
  const maxCalls = entries[0][1].calls || 1;

  tbody.innerHTML = '';
  if (select) {
    select.innerHTML = '<option value="ALL">All Repositories</option>';
  }

  entries.forEach(([rName, rData]) => {
    if (select) {
      const opt = document.createElement('option');
      opt.value = rName;
      opt.textContent = rName;
      select.appendChild(opt);
    }

    const tr = document.createElement('tr');
    tr.className = 'repo-row-item';
    tr.onclick = () => filterByRepo(rName);

    const calls = rData.calls;
    const errs = rData.errors || 0;
    const rel = rData.reliability_pct || (calls ? 100.0 * (calls - errs) / calls : 100.0);
    const wt = rData.worktree;
    const lastStr = rData.last_ts ? new Date(rData.last_ts * 1000).toLocaleString() : '—';
    const pct = (calls / maxCalls) * 100.0;
    const wtBadge = wt ? '<span class="badge badge-wt">worktree</span>' : '<span class="badge badge-repo">repo</span>';
    const relClass = errs === 0 ? 'status-ok' : 'status-err';

    tr.innerHTML =
      '<td>' +
      '  <div class="repo-name-wrap"><strong>' + escapeHtml(rName) + '</strong> ' + wtBadge + '</div>' +
      '  <div class="row-track"><div class="row-track-fill" style="width: ' + pct.toFixed(1) + '%;"></div></div>' +
      '</td>' +
      '<td class="text-right num-col">' + calls.toLocaleString() + '</td>' +
      '<td><span class="badge ' + relClass + '">' + rel.toFixed(1) + '% (' + errs + ' err)</span></td>' +
      '<td class="text-muted num-col">' + lastStr + '</td>';

    tbody.appendChild(tr);
  });
}

function renderAgentsTable(agentAggs, totalCalls) {
  const tbody = document.getElementById('agents-tbody');
  const select = document.getElementById('agent-filter');
  if (!tbody) return;

  const entries = Object.entries(agentAggs);
  if (entries.length === 0) {
    tbody.innerHTML = '<tr><td colspan="4" class="empty-cell">No agent calls recorded</td></tr>';
    return;
  }

  entries.sort((a, b) => b[1].calls - a[1].calls);
  const maxCalls = entries[0][1].calls || 1;

  tbody.innerHTML = '';
  if (select) {
    select.innerHTML = '<option value="ALL">All Agents</option>';
  }

  entries.forEach(([aName, aData]) => {
    if (select) {
      const opt = document.createElement('option');
      opt.value = aName;
      opt.textContent = aName;
      select.appendChild(opt);
    }

    const tr = document.createElement('tr');
    tr.className = 'agent-row-item';
    tr.onclick = () => filterByAgent(aName);

    const calls = aData.calls;
    const errs = aData.errors || 0;
    const rel = aData.reliability_pct || (calls ? 100.0 * (calls - errs) / calls : 100.0);
    const saved = aData.saved_tokens || 0;
    const pct = (calls / maxCalls) * 100.0;
    const share = totalCalls ? (calls / totalCalls * 100.0) : 0.0;
    const shareStr = (share > 0 && share < 0.1) ? '<0.1%' : share.toFixed(1) + '%';
    const relClass = errs === 0 ? 'status-ok' : 'status-err';

    tr.innerHTML =
      '<td>' +
      '  <div class="repo-name-wrap"><span class="agent-tag agent-tag-' + escapeHtml(aName) + '">' + escapeHtml(aName) + '</span> <span class="text-muted" style="font-size: 0.75rem;">(' + shareStr + ')</span></div>' +
      '  <div class="row-track"><div class="row-track-fill row-track-fill-agent" style="width: ' + pct.toFixed(1) + '%;"></div></div>' +
      '</td>' +
      '<td class="text-right num-col">' + calls.toLocaleString() + '</td>' +
      '<td><span class="badge ' + relClass + '">' + rel.toFixed(1) + '% (' + errs + ' err)</span></td>' +
      '<td class="text-right num-col text-muted">~' + saved.toLocaleString() + '</td>';

    tbody.appendChild(tr);
  });
}

function filterByAgent(agentName) {
  const select = document.getElementById('agent-filter');
  if (select) {
    select.value = agentName;
    applyTelemetryFilters();
  }
}

function renderToolsTable(toolAggs, totalCalls) {
  const tbody = document.getElementById('tools-tbody');
  if (!tbody) return;

  const entries = Object.entries(toolAggs);
  if (entries.length === 0) {
    tbody.innerHTML = '<tr><td colspan="3" class="empty-cell">No tool calls recorded</td></tr>';
    return;
  }

  entries.sort((a, b) => b[1].calls - a[1].calls);
  const maxCalls = entries[0][1].calls || 1;

  tbody.innerHTML = '';
  entries.forEach(([tName, tData]) => {
    const tr = document.createElement('tr');
    const calls = tData.calls;
    const pct = (calls / maxCalls) * 100.0;
    const p50Str = (tData.p50 || 0.0).toFixed(1) + 'ms';
    const share = totalCalls ? (calls / totalCalls * 100.0) : 0.0;
    const shareStr = (share > 0 && share < 0.1) ? '<0.1%' : share.toFixed(1) + '%';

    tr.innerHTML =
      '<td>' +
      '  <div class="tool-name-wrap"><code>' + escapeHtml(tName) + '</code> <span class="text-muted" style="font-size: 0.75rem;">(' + shareStr + ')</span></div>' +
      '  <div class="row-track"><div class="row-track-fill" style="width: ' + pct.toFixed(1) + '%;"></div></div>' +
      '</td>' +
      '<td class="text-right num-col">' + calls.toLocaleString() + '</td>' +
      '<td class="text-right num-col text-muted">' + p50Str + '</td>';

    tbody.appendChild(tr);
  });
}

function renderRecentQueries(queries) {
  const tbody = document.getElementById('recent-tbody');
  if (!tbody) return;

  if (queries.length === 0) {
    tbody.innerHTML = '<tr><td colspan="8" class="empty-cell">No recent queries</td></tr>';
    return;
  }

  tbody.innerHTML = '';
  queries.forEach(q => {
    tbody.appendChild(createRecentRow(q));
  });
}

function renderErrorsTriage(errors) {
  const tbody = document.getElementById('errors-tbody');
  if (!tbody) return;

  if (errors.length === 0) {
    tbody.innerHTML = '<tr><td colspan="6" class="empty-cell">Zero critical errors recorded</td></tr>';
    return;
  }

  tbody.innerHTML = '';
  errors.forEach(e => {
    const tr = document.createElement('tr');
    const ts = e.ts ? new Date(e.ts * 1000).toLocaleTimeString() : '—';
    const rName = e.repo_name || 'workspace';
    const aName = e.agent || 'user';
    const latStr = e.latency_us ? (e.latency_us / 1000.0).toFixed(1) + 'ms' : '—';
    const argsStr = JSON.stringify(e.args || {});

    tr.innerHTML =
      '<td class="num-col text-muted">' + ts + '</td>' +
      '<td><span class="repo-tag">' + escapeHtml(rName) + '</span></td>' +
      '<td><code>' + escapeHtml(e.tool || '—') + '</code></td>' +
      '<td><span class="badge status-err">Exit ' + e.exit_code + '</span></td>' +
      '<td class="text-right num-col">' + latStr + '</td>' +
      '<td class="args-cell"><code>' + escapeHtml(argsStr) + '</code></td>';

    tbody.appendChild(tr);
  });
}

function createRecentRow(q) {
  const tr = document.createElement('tr');
  tr.className = 'query-row';
  const rName = q.repo_name || 'workspace';
  const aName = q.agent || 'user';
  const exitCode = parseInt(q.exit_code || 0, 10);
  tr.setAttribute('data-repo', rName);
  tr.setAttribute('data-agent', aName);
  tr.setAttribute('data-code', exitCode);

  const ts = q.ts ? new Date(q.ts * 1000).toLocaleTimeString() : '—';
  const latStr = q.latency_us ? (q.latency_us / 1000.0).toFixed(1) + 'ms' : '—';

  let statusClass = 'status-ok';
  let statusText = 'ok';
  if (exitCode === 1) {
    statusClass = 'status-warn';
    statusText = 'empty';
  } else if (exitCode === 2) {
    statusClass = 'status-ambig';
    statusText = 'ambig';
  } else if (exitCode >= 3) {
    statusClass = 'status-err';
    statusText = 'err ' + exitCode;
  }

  const argsRaw = JSON.stringify(q.args || {});
  const argsShort = argsRaw.length > 85 ? argsRaw.substring(0, 82) + '...' : argsRaw;

  tr.innerHTML =
    '<td class="num-col text-muted">' + ts + '</td>' +
    '<td><span class="repo-tag">' + escapeHtml(rName) + '</span></td>' +
    '<td><span class="agent-tag agent-tag-' + escapeHtml(aName) + '">' + escapeHtml(aName) + '</span></td>' +
    '<td><code>' + escapeHtml(q.tool || '—') + '</code></td>' +
    '<td class="text-right num-col">' + (q.shown || q.result_count || 0) + '</td>' +
    '<td class="text-right num-col">' + latStr + '</td>' +
    '<td><span class="badge ' + statusClass + '">' + statusText + '</span></td>' +
    '<td class="args-cell" title="' + escapeHtml(argsRaw) + '"><code>' + escapeHtml(argsShort) + '</code></td>';

  return tr;
}

function applyTelemetryFilters() {
  const selectedRepo = document.getElementById('repo-filter').value;
  const agentFilterEl = document.getElementById('agent-filter');
  const selectedAgent = agentFilterEl ? agentFilterEl.value : 'ALL';
  const selectedStatus = document.getElementById('status-filter').value;
  const search = document.getElementById('query-search').value.toLowerCase().trim();

  const rows = document.querySelectorAll('#recent-tbody tr.query-row');
  rows.forEach(row => {
    const repo = row.getAttribute('data-repo');
    const agent = row.getAttribute('data-agent');
    const code = row.getAttribute('data-code');
    const text = row.textContent.toLowerCase();

    let matchesRepo = (selectedRepo === 'ALL' || repo === selectedRepo);
    let matchesAgent = (selectedAgent === 'ALL' || agent === selectedAgent);
    let matchesStatus = true;
    if (selectedStatus === '0') matchesStatus = (code === '0');
    else if (selectedStatus === '1') matchesStatus = (code === '1');
    else if (selectedStatus === '2') matchesStatus = (code === '2');
    else if (selectedStatus === 'ERR') matchesStatus = (parseInt(code, 10) >= 3);

    let matchesSearch = (!search || text.includes(search));
    row.style.display = (matchesRepo && matchesAgent && matchesStatus && matchesSearch) ? '' : 'none';
  });
}

function filterByRepo(repoName) {
  const select = document.getElementById('repo-filter');
  if (select) {
    select.value = repoName;
    applyTelemetryFilters();
  }
}

// Anti-Reinvention UI
function populateReuseRepoFilter() {
  const select = document.getElementById('reuse-repo-filter');
  if (!select) return;
  const repos = new Set();
  reuseCandidates.forEach(c => repos.add(c.repo));

  select.innerHTML = '<option value="ALL">All Repositories</option>';
  Array.from(repos).sort().forEach(r => {
    const opt = document.createElement('option');
    opt.value = r;
    opt.textContent = r;
    select.appendChild(opt);
  });
}

function setReuseDetectorFilter(det) {
  activeReuseDetector = det;
  document.querySelectorAll('.filter-pill[data-det]').forEach(btn => {
    btn.classList.toggle('active', btn.getAttribute('data-det') === det);
  });
  selectedReuseIndex = 0;
  renderReuseList();
}

function getFilteredReuseCandidates() {
  const repo = document.getElementById('reuse-repo-filter') ? document.getElementById('reuse-repo-filter').value : 'ALL';
  const search = document.getElementById('reuse-search') ? document.getElementById('reuse-search').value.toLowerCase().trim() : '';

  return reuseCandidates.filter(c => {
    if (repo !== 'ALL' && c.repo !== repo) return false;
    if (activeReuseDetector !== 'ALL' && c.detector !== activeReuseDetector) return false;
    if (search) {
      const matchSym = c.symbol.toLowerCase().includes(search);
      const matchCan = c.canonical.toLowerCase().includes(search);
      const matchEv = (c.shared_evidence || '').toLowerCase().includes(search);
      const matchFile = (c.canonical_file || '').toLowerCase().includes(search);
      if (!matchSym && !matchCan && !matchEv && !matchFile) return false;
    }
    return true;
  });
}

function renderReuseList() {
  const container = document.getElementById('reuse-cards-container');
  if (!container) return;
  const filtered = getFilteredReuseCandidates();

  if (filtered.length === 0) {
    container.innerHTML = '<div class="empty-cell">Zero duplicate candidates match current criteria.<br><span style="font-size: 0.8rem; opacity: 0.7;">Codebase is clean of duplicate helpers.</span></div>';
    renderReuseDossier(null);
    return;
  }

  container.innerHTML = '';
  filtered.forEach((c, idx) => {
    const isSelected = idx === selectedReuseIndex;
    const card = document.createElement('div');
    card.className = 'select-item-card' + (isSelected ? ' selected' : '');
    card.onclick = () => {
      selectedReuseIndex = idx;
      renderReuseList();
    };

    const detBadgeClass = 'badge badge-' + c.detector.toLowerCase();
    const repoBadge = '<span class="badge badge-repo">' + escapeHtml(c.repo) + '</span>';
    const locBadge = '<span class="badge status-warn">-' + c.loc_saving + ' LOC</span>';

    card.innerHTML =
      '<div class="item-card-head">' +
      '  <div style="display:flex; align-items:center; gap:0.4rem;">' +
      '    <span class="' + detBadgeClass + '">' + c.detector + '</span>' +
      '    <span class="badge" style="background:var(--bg-subtle);">' + c.overlap_pct + '% overlap</span>' +
      '    ' + locBadge +
      '  </div>' +
      '  ' + repoBadge +
      '</div>' +
      '<div class="item-key">' + escapeHtml(c.symbol) + '</div>' +
      '<div class="item-snippet">&rarr; canonical: <code>' + escapeHtml(c.canonical) + '</code> <span class="badge status-ok">[callers=' + c.canonical_callers + ']</span></div>' +
      '<div class="item-meta">' +
      '  <span>' + escapeHtml(c.canonical_file) + ':' + c.canonical_line + '</span>' +
      (c.shared_evidence ? '<span>&bull;</span><span>' + escapeHtml(c.shared_evidence) + '</span>' : '') +
      '</div>';

    container.appendChild(card);
  });

  if (filtered[selectedReuseIndex]) {
    renderReuseDossier(filtered[selectedReuseIndex]);
  } else {
    selectedReuseIndex = 0;
    renderReuseDossier(filtered[0] || null);
  }
}

function renderReuseDossier(c) {
  const container = document.getElementById('reuse-dossier-container');
  if (!container) return;
  if (!c) {
    container.innerHTML = '<div class="empty-cell" style="padding-top: 5rem;">Select a duplication candidate to inspect side-by-side source code.</div>';
    return;
  }

  const detBadgeClass = 'badge badge-' + c.detector.toLowerCase();
  const symSnippet = c.candidate_snippet || '(Source snippet unavailable)';
  const canSnippet = c.canonical_snippet || '(Source snippet unavailable)';

  container.innerHTML =
    '<div class="dossier-head">' +
    '  <div style="display:flex; justify-content:space-between; align-items:center; margin-bottom: 0.5rem;">' +
    '    <div style="display:flex; gap:0.45rem; align-items:center;">' +
    '      <span class="' + detBadgeClass + '">' + c.detector + '</span>' +
    '      <span class="badge status-ok">' + c.overlap_pct + '% OVERLAP</span>' +
    '      <span class="badge badge-repo">' + escapeHtml(c.repo) + '</span>' +
    '    </div>' +
    '    <span class="badge status-warn">Save ~' + c.loc_saving + ' LOC</span>' +
    '  </div>' +
    '  <div class="dossier-title">' + escapeHtml(c.symbol) + '</div>' +
    '  <div class="text-muted" style="font-size:0.82rem;">Duplicates Authoritative Target: <strong>' + escapeHtml(c.canonical) + '</strong> <span class="badge status-ok">[callers=' + c.canonical_callers + ']</span></div>' +
    '</div>' +
    '<div class="prevention-banner">' +
    '  <strong>What This Prevents:</strong> ' + escapeHtml(c.prevention_narrative) +
    '</div>' +
    (c.shared_evidence ? '<div class="dossier-section-title">Shared Footprint &amp; Overlap Evidence</div><div class="chips-row"><span class="chip chip-tag">' + escapeHtml(c.shared_evidence) + '</span></div>' : '') +
    '<div class="dossier-section-title">Side-by-Side Source Code Comparison</div>' +
    '<div class="side-by-side-grid">' +
    '  <div class="code-panel">' +
    '    <div class="code-panel-head">' +
    '      <span class="badge status-warn">DUPLICATE CANDIDATE</span>' +
    '      <span>' + escapeHtml(c.candidate_info ? c.candidate_info.file : '') + ':' + (c.candidate_info ? c.candidate_info.line_start : 0) + '-' + (c.candidate_info ? c.candidate_info.line_end : 0) + '</span>' +
    '    </div>' +
    '    <pre><code>' + escapeHtml(symSnippet) + '</code></pre>' +
    '  </div>' +
    '  <div class="code-panel">' +
    '    <div class="code-panel-head">' +
    '      <span class="badge status-ok">CANONICAL REUSE TARGET</span>' +
    '      <span>' + escapeHtml(c.canonical_info ? c.canonical_info.file : '') + ':' + (c.canonical_info ? c.canonical_info.line_start : 0) + '-' + (c.canonical_info ? c.canonical_info.line_end : 0) + '</span>' +
    '    </div>' +
    '    <pre><code>' + escapeHtml(canSnippet) + '</code></pre>' +
    '  </div>' +
    '</div>';
}

function renderPrewriteTable() {
  const tbody = document.getElementById('prewrite-tbody');
  if (!tbody) return;
  if (!prewriteSearches || prewriteSearches.length === 0) {
    tbody.innerHTML = '<tr><td colspan="6" class="empty-cell">No pre-write helper searches recorded yet</td></tr>';
    return;
  }

  tbody.innerHTML = '';
  prewriteSearches.forEach(s => {
    const tr = document.createElement('tr');
    const ts = s.ts ? new Date(s.ts * 1000).toLocaleTimeString() : '—';
    const latStr = s.latency_us ? (s.latency_us / 1000.0).toFixed(1) + 'ms' : '—';
    tr.innerHTML =
      '<td class="num-col text-muted">' + ts + '</td>' +
      '<td><span class="repo-tag">' + escapeHtml(s.repo_name) + '</span></td>' +
      '<td><code>' + escapeHtml(s.tool) + '</code></td>' +
      '<td class="args-cell"><code>' + escapeHtml(s.terms || '(all paths)') + '</code></td>' +
      '<td class="text-right num-col">' + s.shown + '</td>' +
      '<td class="text-right num-col">' + latStr + '</td>';
    tbody.appendChild(tr);
  });
}

// Agent Memory UI
function populateMemoryStoreFilter() {
  const select = document.getElementById('mem-store-filter');
  if (!select) return;
  const stores = new Set();
  allMemories.forEach(m => stores.add(m.store));

  select.innerHTML = '<option value="ALL">All Stores</option>';
  Array.from(stores).sort().forEach(s => {
    const opt = document.createElement('option');
    opt.value = s;
    opt.textContent = s;
    select.appendChild(opt);
  });
}

function getFilteredMemories() {
  const store = document.getElementById('mem-store-filter') ? document.getElementById('mem-store-filter').value : 'ALL';
  const kind = document.getElementById('mem-kind-filter') ? document.getElementById('mem-kind-filter').value : 'ALL';
  const search = document.getElementById('mem-search') ? document.getElementById('mem-search').value.toLowerCase().trim() : '';
  const showSuperseded = document.getElementById('mem-show-superseded') ? document.getElementById('mem-show-superseded').checked : false;

  return allMemories.filter(m => {
    if (!showSuperseded && !m.is_current) return false;
    if (store !== 'ALL' && m.store !== store) return false;
    if (kind !== 'ALL' && m.kind !== kind) return false;
    if (search) {
      const matchKey = m.key.toLowerCase().includes(search);
      const matchBody = m.body.toLowerCase().includes(search);
      const matchTags = (m.tags || []).some(t => t.toLowerCase().includes(search));
      const matchRefs = (m.refs || []).some(r => r.toLowerCase().includes(search));
      if (!matchKey && !matchBody && !matchTags && !matchRefs) return false;
    }
    return true;
  });
}

function renderMemoryList() {
  const container = document.getElementById('memory-cards-container');
  if (!container) return;
  const filtered = getFilteredMemories();

  if (filtered.length === 0) {
    container.innerHTML = '<div class="empty-cell">No memory notes match the filter.<br><span style="font-size: 0.8rem; opacity: 0.7;">Agents write memories via <code>aw mem write --body "..." --kind gotcha &lt;key&gt;</code></span></div>';
    renderMemoryDossier(null);
    return;
  }

  container.innerHTML = '';
  filtered.forEach((m, idx) => {
    const isSelected = idx === selectedMemoryIndex;
    const card = document.createElement('div');
    card.className = 'select-item-card' + (isSelected ? ' selected' : '');
    card.onclick = () => {
      selectedMemoryIndex = idx;
      renderMemoryList();
    };

    const kindClass = 'badge status-' + (m.kind === 'decision' ? 'ambig' : m.kind === 'gotcha' ? 'warn' : m.kind === 'fact' ? 'ok' : 'err');
    const timeStr = m.created_at ? new Date(m.created_at * 1000).toLocaleDateString() : '—';
    const storeBadge = '<span class="badge badge-repo">' + escapeHtml(m.store) + '</span>';
    const revPill = '<span class="badge" style="background: var(--bg-subtle);">r' + m.rev + '</span>';
    const recallsBadge = m.recall_count > 0 ? '<span class="badge status-ok">' + m.recall_count + ' recalls</span>' : '';
    const refBadge = m.ref_status === 'broken'
      ? '<span class="badge status-err">[broken-ref]</span>'
      : m.ref_status === 'modified'
      ? '<span class="badge status-warn">[modified-ref]</span>'
      : '';

    card.innerHTML =
      '<div class="item-card-head">' +
      '  <div style="display:flex; align-items:center; gap:0.4rem;">' +
      '    <span class="' + kindClass + '">' + m.kind.toUpperCase() + '</span>' +
      '    ' + revPill +
      '    ' + recallsBadge +
      '    ' + refBadge +
      '  </div>' +
      '  ' + storeBadge +
      '</div>' +
      '<div class="item-key">' + escapeHtml(m.key) + '</div>' +
      '<div class="item-snippet">' + escapeHtml(m.body) + '</div>' +
      '<div class="item-meta">' +
      '  <span>' + m.scope + '</span>' +
      '  <span>&bull;</span>' +
      '  <span>' + timeStr + '</span>' +
      (m.refs && m.refs.length ? '<span>&bull;</span><span>' + m.refs.length + ' refs</span>' : '') +
      '</div>';

    container.appendChild(card);
  });

  if (filtered[selectedMemoryIndex]) {
    renderMemoryDossier(filtered[selectedMemoryIndex]);
  } else {
    selectedMemoryIndex = 0;
    renderMemoryDossier(filtered[0] || null);
  }
}

function renderMemoryDossier(m) {
  const container = document.getElementById('memory-dossier-container');
  if (!container) return;
  if (!m) {
    container.innerHTML = '<div class="empty-cell" style="padding-top: 5rem;">Select a memory note from the list to view full dossier.</div>';
    return;
  }

  const kindClass = 'badge status-' + (m.kind === 'decision' ? 'ambig' : m.kind === 'gotcha' ? 'warn' : m.kind === 'fact' ? 'ok' : 'err');
  const dateStr = m.created_at ? new Date(m.created_at * 1000).toLocaleString() : '—';
  const lastRecalledStr = m.last_recalled_ts ? new Date(m.last_recalled_ts * 1000).toLocaleString() : 'Never queried in current session';

  let tagsHtml = '';
  if (m.tags && m.tags.length) {
    tagsHtml = '<div class="dossier-section-title">Tags</div><div class="chips-row">';
    m.tags.forEach(t => { tagsHtml += '<span class="chip chip-tag">#' + escapeHtml(t) + '</span>'; });
    tagsHtml += '</div>';
  }

  let refsHtml = '';
  if (m.resolved_refs && m.resolved_refs.length) {
    refsHtml = '<div class="dossier-section-title">Guarded Code References (.aw/index.db verified)</div><div class="chips-row">';
    m.resolved_refs.forEach(r => {
      const verClass = !r.verified ? 'chip-ref broken' : r.modified ? 'chip-ref modified' : 'chip-ref verified';
      const verBadge = !r.verified ? ' [BROKEN REF]' : r.modified ? ' [MODIFIED REF]' : ' [VERIFIED ' + r.type.toUpperCase() + ']';
      const locStr = r.location ? ' &middot; ' + escapeHtml(r.location) : '';
      refsHtml += '<span class="' + verClass + '"><strong>' + escapeHtml(r.ref) + '</strong>' + verBadge + locStr + '</span>';
    });
    refsHtml += '</div>';
  } else if (m.refs && m.refs.length) {
    refsHtml = '<div class="dossier-section-title">Referenced Symbols &amp; Paths</div><div class="chips-row">';
    m.refs.forEach(r => { refsHtml += '<span class="chip chip-ref">' + escapeHtml(r) + '</span>'; });
    refsHtml += '</div>';
  }

  const touchBtn = m.ref_status === 'modified'
    ? '<button class="btn-touch" style="padding: 0.15rem 0.5rem; font-size: 0.72rem; margin-left: 0.5rem;" onclick="touchMemory(\'' + escapeHtml(m.key) + '\', ' + (m.store === 'global') + ')">Touch Anchor</button>'
    : '';

  container.innerHTML =
    '<div class="dossier-head">' +
    '  <div style="display:flex; justify-content:space-between; align-items:center; margin-bottom: 0.5rem;">' +
    '    <div style="display:flex; gap:0.45rem; align-items:center;">' +
    '      <span class="' + kindClass + '">' + m.kind.toUpperCase() + '</span>' +
    '      <span class="badge" style="background:var(--bg-subtle);">Rev ' + m.rev + '</span>' +
    '      <span class="badge badge-repo">' + escapeHtml(m.store) + '</span>' +
    '      ' + touchBtn +
    '    </div>' +
    '    <span class="text-muted" style="font-size:0.78rem;">' + dateStr + '</span>' +
    '  </div>' +
    '  <div class="dossier-title">' + escapeHtml(m.key) + '</div>' +
    '  <div class="text-muted" style="font-size:0.8rem;">Scope: <strong>' + m.scope + '</strong> &middot; Source: <code>' + m.source + '</code></div>' +
    '</div>' +
    '<div class="recall-banner">' +
    '  <div><strong>' + escapeHtml(m.prev_category || 'KNOWLEDGE GUARD') + ':</strong> ' + escapeHtml(m.prev_detail || '') + '</div>' +
    '  <div style="font-family:var(--font-mono); font-size:0.78rem;">Recalls: <strong>' + m.recall_count + '</strong> &middot; Last: ' + lastRecalledStr + '</div>' +
    '</div>' +
    '<div class="dossier-section-title">Memory Content</div>' +
    '<div class="dossier-body">' + escapeHtml(m.body) + '</div>' +
    refsHtml +
    tagsHtml;
}

let liveEventCount = 0;

// Live Server-Sent Events (SSE)
function initEventSource() {
  const liveBadge = document.getElementById('live-indicator');
  if (liveBadge) liveBadge.style.display = 'inline-block';

  const source = new EventSource('/events');
  source.onmessage = function(e) {
    try {
      const q = JSON.parse(e.data);
      handleLiveEvent(q);
      liveEventCount++;
      if (liveEventCount % 5 === 0) {
        loadSummaryData();
      }
      if (q.tool && q.tool.startsWith('mem_')) {
        loadMemoriesData();
      }
      if (q.tool && (q.tool === 'reuse' || q.tool === 'search')) {
        loadReuseData();
      }
    } catch(err) {}
  };
}

function handleLiveEvent(q) {
  totalCalls++;
  const exitCode = parseInt(q.exit_code || 0, 10);
  if (exitCode >= 3) totalErrors++;

  const totalCallsEl = document.getElementById('metric-total-calls');
  if (totalCallsEl) totalCallsEl.textContent = totalCalls.toLocaleString();

  const relEl = document.getElementById('metric-reliability');
  if (relEl) {
    const rel = totalCalls > 0 ? (100.0 * (totalCalls - totalErrors) / totalCalls).toFixed(1) : "100.0";
    relEl.textContent = rel + "% clean (" + totalErrors + " errs)";
  }

  const recentTbody = document.getElementById('recent-tbody');
  if (recentTbody) {
    const row = createRecentRow(q);
    recentTbody.insertBefore(row, recentTbody.firstChild);
    while (recentTbody.children.length > 50) {
      recentTbody.removeChild(recentTbody.lastChild);
    }
  }
}

function escapeHtml(str) {
  return String(str || '')
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}

window.addEventListener('DOMContentLoaded', () => {
  initTheme();
  loadSummaryData();
  loadReuseData();
  loadMemoriesData();
  initEventSource();
});
