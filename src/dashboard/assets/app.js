// eg Native Rust Dashboard Client (Zero External Dependencies)

const GRAPH_TOOLS = ['callers', 'callees', 'path', 'context', 'impact', 'symbols', 'imports', 'build'];

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
  const saved = localStorage.getItem('eg-theme') || 'light';
  document.documentElement.setAttribute('data-theme', saved);
  updateThemeButtonText(saved);
}

function toggleTheme() {
  const current = document.documentElement.getAttribute('data-theme') || 'light';
  const next = current === 'light' ? 'dark' : 'light';
  document.documentElement.setAttribute('data-theme', next);
  localStorage.setItem('eg-theme', next);
  updateThemeButtonText(next);
}

function updateThemeButtonText(theme) {
  const btn = document.getElementById('theme-toggle-btn');
  if (btn) btn.textContent = 'Theme: ' + (theme === 'dark' ? 'Dark' : 'Light');
}

let activeTab = 'telemetry';

// Tab Switching
function switchTab(tab) {
  activeTab = tab;
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
    if (latestSummaryData) {
      renderReuseReposTable(latestSummaryData.repo_aggs || {});
      renderReuseAgentsTable(latestSummaryData.agent_aggs || {});
    }
    loadReuseData();
    loadSummaryData();
  } else if (tab === 'memory') {
    populateMemoryStoreFilter();
    renderMemoryList();
    if (latestSummaryData) {
      renderMemoryReposTable(latestSummaryData.repo_aggs || {});
      renderMemoryAgentsTable(latestSummaryData.agent_aggs || {});
    }
    loadMemoriesData();
    loadSummaryData();
  } else {
    loadSummaryData();
  }
}

// Data Fetching
function loadSummaryData() {
  fetch('/api/summary')
    .then(r => r.json())
    .then(data => {
      latestSummaryData = data;
      totalCalls = data.graph_calls !== undefined ? data.graph_calls : (data.total_calls || 0);
      totalErrors = data.graph_errors !== undefined ? data.graph_errors : (data.error_count || 0);

      const totalCallsEl = document.getElementById('metric-total-calls');
      if (totalCallsEl) totalCallsEl.textContent = totalCalls.toLocaleString();

      const relEl = document.getElementById('metric-reliability');
      if (relEl) {
        const rel = data.graph_reliability_pct !== undefined ? data.graph_reliability_pct : (data.reliability_pct || 100.0);
        relEl.textContent = totalErrors > 0
          ? rel.toFixed(1) + '% clean (' + totalErrors + ' errs)'
          : 'No errors recorded';
      }

      const p50El = document.getElementById('metric-p50');
      if (p50El) {
        const p50 = data.graph_p50_ms !== undefined ? data.graph_p50_ms : data.p50_ms;
        p50El.textContent = (p50 || 0.0).toFixed(1) + 'ms';
      }

      const latSubEl = document.getElementById('metric-latency-sub');
      if (latSubEl) {
        const p95 = data.graph_p95_ms !== undefined ? data.graph_p95_ms : data.p95_ms;
        const avg = data.graph_avg_ms !== undefined ? data.graph_avg_ms : data.avg_ms;
        latSubEl.textContent = 'p95 = ' + (p95 || 0.0).toFixed(1) + 'ms · avg = ' + (avg || 0.0).toFixed(1) + 'ms';
      }

      const tokEl = document.getElementById('metric-tokens');
      if (tokEl) {
        const tokens = data.graph_saved_tokens !== undefined ? data.graph_saved_tokens : (data.saved_tokens || 0);
        tokEl.textContent = '~' + tokens.toLocaleString();
      }

      const dolEl = document.getElementById('metric-dollars');
      if (dolEl) {
        const dollars = data.graph_saved_dollars !== undefined ? data.graph_saved_dollars : (data.saved_dollars || 0.0);
        dolEl.textContent = '~$' + dollars.toFixed(2) + ' avoided vs full reads';
      }

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
          : 'Multi-repository graph index';
      }

      // Render Repositories Table
      renderReposTable(data.repo_aggs || {});
      // Render Agents Table
      renderAgentsTable(data.agent_aggs || {}, totalCalls);
      // Render Anti-Reinvention Repos & Agents
      renderReuseReposTable(data.repo_aggs || {});
      renderReuseAgentsTable(data.agent_aggs || {});
      // Render Memory Repos & Agents
      renderMemoryReposTable(data.repo_aggs || {});
      renderMemoryAgentsTable(data.agent_aggs || {});
      // Render Tools Table
      renderToolsTable(data.tool_aggs || {}, totalCalls);
      // Render Recent Queries (Graph only)
      renderRecentQueries(data.graph_recent_queries || data.recent_queries || []);
      // Render Errors Triage (Graph only)

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

      const summary = data.summary || {};
      const totalCandidates = summary.total_candidates !== undefined ? summary.total_candidates : reuseCandidates.length;

      const kpiCount = document.getElementById('reuse-kpi-count');
      if (kpiCount) kpiCount.textContent = totalCandidates.toLocaleString();

      const kpiCountSub = document.getElementById('reuse-kpi-count-sub');
      if (kpiCountSub) {
        if (totalCandidates > reuseCandidates.length) {
          kpiCountSub.textContent = 'Top ' + reuseCandidates.length + ' of ' + totalCandidates.toLocaleString() + ' duplicate twins';
        } else {
          kpiCountSub.textContent = 'Syntactic & structural duplicate twins';
        }
      }

      const kpiLoc = document.getElementById('reuse-kpi-loc');
      if (kpiLoc) kpiLoc.textContent = '~' + (summary.preventable_loc || 0).toLocaleString() + ' LOC';

      const feedCount = document.getElementById('reuse-feed-count');
      if (feedCount) {
        const count = summary.prewrite_count !== undefined ? summary.prewrite_count : prewriteSearches.length;
        feedCount.textContent = count.toLocaleString();
      }

      const counts = summary.detector_counts || {};
      updateDetectorPills(counts, reuseCandidates.length);
      populateReuseRepoFilter();
      renderReuseList();
      renderPrewriteTable();
    })
    .catch(() => {});
}

function updateDetectorPills(counts, total) {
  const pills = document.querySelectorAll('#reuse-detector-pills .filter-pill');
  pills.forEach(pill => {
    const det = pill.getAttribute('data-det');
    if (det === 'ALL') {
      pill.textContent = 'All (' + total + ')';
    } else if (counts && counts[det] !== undefined) {
      const label = det === 'D1' ? 'D1 Callee' : det === 'D2' ? 'D2 Interface' : det === 'D3' ? 'D3 Bypass' : 'D4 Wrapper';
      pill.textContent = label + ' (' + counts[det] + ')';
    }
  });
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

  // KPI 1: Session Recall Rate
  // The rate only means something when callers tag their sessions: without a
  // tag every run collapses into the synthetic `default` bucket.
  const taggedSessions = latestSummaryData ? (latestSummaryData.session_tagged_count || 0) : 0;
  const recallPct = taggedSessions === 0
    ? '—'
    : (latestSummaryData ? (latestSummaryData.session_recall_pct || 0.0).toFixed(1) + '%' : '0.0%');
  const recallEl = document.getElementById('mem-kpi-recall-pct');
  if (recallEl) recallEl.textContent = recallPct;
  const recallSubEl = document.getElementById('mem-kpi-recall-pct-sub');
  if (recallSubEl) {
    if (taggedSessions === 0) {
      recallSubEl.textContent = 'Set ENGRAM_SESSION to measure';
    } else {
      const memSessions = latestSummaryData ? (latestSummaryData.memory_session_count || 0) : 0;
      const totalSessions = latestSummaryData ? (latestSummaryData.session_count || 0) : 0;
      recallSubEl.textContent = memSessions + ' of ' + totalSessions + ' sessions';
    }
  }

  // KPI 2: Anchor Currency Rate
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
      : 'Referenced symbol or file path is missing from current code index (.engram/index.db).';
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

  const entries = Object.entries(repoAggs).map(([rName, rData]) => {
    const calls = rData.graph_calls !== undefined ? rData.graph_calls : rData.calls;
    const errs = rData.graph_errors !== undefined ? rData.graph_errors : (rData.errors || 0);
    const rel = rData.graph_reliability_pct !== undefined ? rData.graph_reliability_pct : (rData.reliability_pct || (calls ? 100.0 * (calls - errs) / calls : 100.0));
    return [rName, { ...rData, calls, errors: errs, reliability_pct: rel }];
  }).filter(([_, rData]) => rData.calls > 0);

  if (entries.length === 0) {
    tbody.innerHTML = '<tr><td colspan="4" class="empty-cell">No graph workspace activity recorded</td></tr>';
    return;
  }

  entries.sort((a, b) => b[1].calls - a[1].calls);
  const maxCalls = entries[0][1].calls || 1;

  tbody.innerHTML = '';
  const curRepoVal = select ? select.value : 'ALL';
  if (select) {
    select.innerHTML = '<option value="ALL">All Repositories</option>';
  }

  entries.forEach(([rName, rData]) => {
    if (select) {
      const opt = document.createElement('option');
      opt.value = rData.display || rName;
      opt.textContent = rName;
      select.appendChild(opt);
    }

    const tr = document.createElement('tr');
    tr.className = 'repo-row-item';
    tr.onclick = () => filterByRepo(rName);

    const calls = rData.calls;
    const errs = rData.errors || 0;
    const rel = rData.reliability_pct;
    const wt = rData.worktree;
    const lastStr = rData.last_ts ? new Date(rData.last_ts * 1000).toLocaleString() : '—';
    const pct = (calls / maxCalls) * 100.0;
    const wtBadge = wt ? '<span class="badge badge-wt">worktree</span>' : '<span class="badge badge-repo">repo</span>';
    const relClass = errs === 0 ? 'status-ok' : 'status-err';

    tr.innerHTML =
      '<td>' +
      '  <div class="repo-name-wrap"><strong>' + escapeHtml(rData.display || rName) + '</strong> ' + wtBadge + '</div>' +
      '  <div class="row-track"><div class="row-track-fill" style="width: ' + pct.toFixed(1) + '%;"></div></div>' +
      '</td>' +
      '<td class="text-right num-col">' + calls.toLocaleString() + '</td>' +
      '<td><span class="badge ' + relClass + '">' + rel.toFixed(1) + '% (' + errs + ' err)</span></td>' +
      '<td class="text-muted num-col">' + lastStr + '</td>';

    tbody.appendChild(tr);
  });
  if (select && curRepoVal) {
    select.value = curRepoVal;
  }
}

function renderAgentsTable(agentAggs, totalCalls) {
  const tbody = document.getElementById('agents-tbody');
  const select = document.getElementById('agent-filter');
  if (!tbody) return;

  const entries = Object.entries(agentAggs).map(([aName, aData]) => {
    const calls = aData.graph_calls !== undefined ? aData.graph_calls : aData.calls;
    const errs = aData.graph_errors !== undefined ? aData.graph_errors : (aData.errors || 0);
    const saved = aData.graph_saved_tokens !== undefined ? aData.graph_saved_tokens : (aData.saved_tokens || 0);
    const rel = aData.graph_reliability_pct !== undefined ? aData.graph_reliability_pct : (aData.reliability_pct || (calls ? 100.0 * (calls - errs) / calls : 100.0));
    return [aName, { ...aData, calls, errors: errs, saved_tokens: saved, reliability_pct: rel }];
  }).filter(([_, aData]) => aData.calls > 0);

  if (entries.length === 0) {
    tbody.innerHTML = '<tr><td colspan="4" class="empty-cell">No agent graph navigation recorded</td></tr>';
    return;
  }

  entries.sort((a, b) => b[1].calls - a[1].calls);
  const maxCalls = entries[0][1].calls || 1;

  tbody.innerHTML = '';
  const curAgentVal = select ? select.value : 'ALL';
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
    const rel = aData.reliability_pct;
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
  if (select && curAgentVal) {
    select.value = curAgentVal;
  }
}

function filterByAgent(agentName) {
  const select = document.getElementById('agent-filter');
  if (select) {
    select.value = agentName;
    applyTelemetryFilters();
  }
}

function renderReuseReposTable(repoAggs) {
  const tbody = document.getElementById('reuse-repos-tbody');
  if (!tbody) return;

  const entries = Object.entries(repoAggs)
    .filter(([_, rData]) => (rData.reuse_calls || 0) > 0);

  if (entries.length === 0) {
    tbody.innerHTML = '<tr><td colspan="4" class="empty-cell">No anti-reinvention workspace activity recorded</td></tr>';
    return;
  }

  entries.sort((a, b) => (b[1].reuse_calls || 0) - (a[1].reuse_calls || 0));
  const maxCalls = entries[0][1].reuse_calls || 1;

  tbody.innerHTML = '';
  entries.forEach(([rName, rData]) => {
    const tr = document.createElement('tr');
    tr.className = 'repo-row-item';
    tr.onclick = () => filterReuseByRepo(rName);

    const calls = rData.reuse_calls || 0;
    const hits = rData.reuse_hits || 0;
    const saved = rData.reuse_saved_tokens || 0;
    const wt = rData.worktree;
    const pct = (calls / maxCalls) * 100.0;
    const wtBadge = wt ? '<span class="badge badge-wt">worktree</span>' : '<span class="badge badge-repo">repo</span>';
    const hitBadge = hits > 0 ? '<span class="badge status-ok">' + hits.toLocaleString() + ' hits</span>' : '<span class="text-muted">0</span>';

    tr.innerHTML =
      '<td>' +
      '  <div class="repo-name-wrap"><strong>' + escapeHtml(rData.display || rName) + '</strong> ' + wtBadge + '</div>' +
      '  <div class="row-track"><div class="row-track-fill" style="width: ' + pct.toFixed(1) + '%;"></div></div>' +
      '</td>' +
      '<td class="text-right num-col">' + calls.toLocaleString() + '</td>' +
      '<td class="text-right num-col">' + hitBadge + '</td>' +
      '<td class="text-right num-col text-muted">~' + saved.toLocaleString() + '</td>';

    tbody.appendChild(tr);
  });
}

function renderReuseAgentsTable(agentAggs) {
  const tbody = document.getElementById('reuse-agents-tbody');
  if (!tbody) return;

  const entries = Object.entries(agentAggs)
    .filter(([_, aData]) => (aData.reuse_calls || 0) > 0);

  if (entries.length === 0) {
    tbody.innerHTML = '<tr><td colspan="4" class="empty-cell">No agent anti-reinvention activity recorded</td></tr>';
    return;
  }

  const totalReuseCalls = entries.reduce((acc, [_, aData]) => acc + (aData.reuse_calls || 0), 0);
  entries.sort((a, b) => (b[1].reuse_calls || 0) - (a[1].reuse_calls || 0));
  const maxCalls = entries[0][1].reuse_calls || 1;

  tbody.innerHTML = '';
  entries.forEach(([aName, aData]) => {
    const tr = document.createElement('tr');
    tr.className = 'agent-row-item';
    tr.onclick = () => {
      switchTab('graph');
      filterByAgent(aName);
    };

    const calls = aData.reuse_calls || 0;
    const hits = aData.reuse_hits || 0;
    const saved = aData.reuse_saved_tokens || 0;
    const pct = (calls / maxCalls) * 100.0;
    const share = totalReuseCalls ? (calls / totalReuseCalls * 100.0) : 0.0;
    const shareStr = (share > 0 && share < 0.1) ? '<0.1%' : share.toFixed(1) + '%';
    const hitBadge = hits > 0 ? '<span class="badge status-ok">' + hits.toLocaleString() + ' hits</span>' : '<span class="text-muted">0</span>';

    tr.innerHTML =
      '<td>' +
      '  <div class="repo-name-wrap"><span class="agent-tag agent-tag-' + escapeHtml(aName) + '">' + escapeHtml(aName) + '</span> <span class="text-muted" style="font-size: 0.75rem;">(' + shareStr + ')</span></div>' +
      '  <div class="row-track"><div class="row-track-fill row-track-fill-agent" style="width: ' + pct.toFixed(1) + '%;"></div></div>' +
      '</td>' +
      '<td class="text-right num-col">' + calls.toLocaleString() + '</td>' +
      '<td class="text-right num-col">' + hitBadge + '</td>' +
      '<td class="text-right num-col text-muted">~' + saved.toLocaleString() + '</td>';

    tbody.appendChild(tr);
  });
}

function filterReuseByRepo(repoName) {
  const select = document.getElementById('reuse-repo-filter');
  if (select) {
    let exists = false;
    for (let i = 0; i < select.options.length; i++) {
      if (select.options[i].value === repoName) {
        exists = true;
        break;
      }
    }
    if (!exists) {
      const opt = document.createElement('option');
      opt.value = repoName;
      opt.textContent = repoName;
      select.appendChild(opt);
    }
    select.value = (select.value === repoName) ? 'ALL' : repoName;
    renderReuseList();
  }
}

function renderMemoryReposTable(repoAggs) {
  const tbody = document.getElementById('mem-repos-tbody');
  if (!tbody) return;

  const entries = Object.entries(repoAggs)
    .filter(([_, rData]) => (rData.memory_calls || 0) > 0);

  if (entries.length === 0) {
    tbody.innerHTML = '<tr><td colspan="4" class="empty-cell">No memory workspace activity recorded</td></tr>';
    return;
  }

  entries.sort((a, b) => (b[1].memory_calls || 0) - (a[1].memory_calls || 0));
  const maxCalls = entries[0][1].memory_calls || 1;

  tbody.innerHTML = '';
  entries.forEach(([rName, rData]) => {
    const tr = document.createElement('tr');
    tr.className = 'repo-row-item';
    tr.onclick = () => filterMemoryByRepo(rName);

    const calls = rData.memory_calls || 0;
    const errs = rData.memory_errors || 0;
    const reads = rData.memory_reads || 0;
    const writes = rData.memory_writes || 0;
    const rel = rData.memory_reliability_pct !== undefined ? rData.memory_reliability_pct : (calls ? 100.0 * (calls - errs) / calls : 100.0);
    const wt = rData.worktree;
    const pct = (calls / maxCalls) * 100.0;
    const wtBadge = wt ? '<span class="badge badge-wt">worktree</span>' : '<span class="badge badge-repo">repo</span>';
    const relClass = errs === 0 ? 'status-ok' : 'status-err';

    tr.innerHTML =
      '<td>' +
      '  <div class="repo-name-wrap"><strong>' + escapeHtml(rData.display || rName) + '</strong> ' + wtBadge + '</div>' +
      '  <div class="row-track"><div class="row-track-fill" style="width: ' + pct.toFixed(1) + '%;"></div></div>' +
      '</td>' +
      '<td class="text-right num-col">' + calls.toLocaleString() + '</td>' +
      '<td class="text-right num-col text-muted">' + reads.toLocaleString() + ' r · ' + writes.toLocaleString() + ' w</td>' +
      '<td><span class="badge ' + relClass + '">' + rel.toFixed(1) + '% (' + errs + ' err)</span></td>';

    tbody.appendChild(tr);
  });
}

function renderMemoryAgentsTable(agentAggs) {
  const tbody = document.getElementById('mem-agents-tbody');
  if (!tbody) return;

  const entries = Object.entries(agentAggs)
    .filter(([_, aData]) => (aData.memory_calls || 0) > 0);

  if (entries.length === 0) {
    tbody.innerHTML = '<tr><td colspan="4" class="empty-cell">No agent memory activity recorded</td></tr>';
    return;
  }

  const totalMemCalls = entries.reduce((acc, [_, aData]) => acc + (aData.memory_calls || 0), 0);
  entries.sort((a, b) => (b[1].memory_calls || 0) - (a[1].memory_calls || 0));
  const maxCalls = entries[0][1].memory_calls || 1;

  tbody.innerHTML = '';
  entries.forEach(([aName, aData]) => {
    const tr = document.createElement('tr');
    tr.className = 'agent-row-item';
    tr.onclick = () => {
      switchTab('graph');
      filterByAgent(aName);
    };

    const calls = aData.memory_calls || 0;
    const errs = aData.memory_errors || 0;
    const reads = aData.memory_reads || 0;
    const writes = aData.memory_writes || 0;
    const rel = aData.memory_reliability_pct !== undefined ? aData.memory_reliability_pct : (calls ? 100.0 * (calls - errs) / calls : 100.0);
    const pct = (calls / maxCalls) * 100.0;
    const share = totalMemCalls ? (calls / totalMemCalls * 100.0) : 0.0;
    const shareStr = (share > 0 && share < 0.1) ? '<0.1%' : share.toFixed(1) + '%';
    const relClass = errs === 0 ? 'status-ok' : 'status-err';

    tr.innerHTML =
      '<td>' +
      '  <div class="repo-name-wrap"><span class="agent-tag agent-tag-' + escapeHtml(aName) + '">' + escapeHtml(aName) + '</span> <span class="text-muted" style="font-size: 0.75rem;">(' + shareStr + ')</span></div>' +
      '  <div class="row-track"><div class="row-track-fill row-track-fill-agent" style="width: ' + pct.toFixed(1) + '%;"></div></div>' +
      '</td>' +
      '<td class="text-right num-col">' + calls.toLocaleString() + '</td>' +
      '<td class="text-right num-col text-muted">' + reads.toLocaleString() + ' r · ' + writes.toLocaleString() + ' w</td>' +
      '<td><span class="badge ' + relClass + '">' + rel.toFixed(1) + '% (' + errs + ' err)</span></td>';

    tbody.appendChild(tr);
  });
}

function filterMemoryByRepo(repoName) {
  const select = document.getElementById('mem-store-filter');
  if (select) {
    let exists = false;
    for (let i = 0; i < select.options.length; i++) {
      if (select.options[i].value === repoName) {
        exists = true;
        break;
      }
    }
    if (!exists) {
      const opt = document.createElement('option');
      opt.value = repoName;
      opt.textContent = repoName;
      select.appendChild(opt);
    }
    select.value = (select.value === repoName) ? 'ALL' : repoName;
    renderMemoryList();
  }
}

function renderToolsTable(toolAggs, totalCalls) {
  const tbody = document.getElementById('tools-tbody');
  if (!tbody) return;

  const entries = Object.entries(toolAggs).filter(([tName]) => GRAPH_TOOLS.includes(tName));
  if (entries.length === 0) {
    tbody.innerHTML = '<tr><td colspan="3" class="empty-cell">No graph tool calls recorded</td></tr>';
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

  const graphQueries = queries.filter(q => GRAPH_TOOLS.includes(q.tool));
  if (graphQueries.length === 0) {
    tbody.innerHTML = '<tr><td colspan="8" class="empty-cell">No graph navigation queries recorded</td></tr>';
    return;
  }

  tbody.innerHTML = '';
  graphQueries.forEach(q => {
    tbody.appendChild(createRecentRow(q));
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
    '<td><code>' + escapeHtml(q.tool || '—') + '</code>' +
      (q.judged ? ' <span class="badge badge-judge" title="eg-judge dispatched">judge</span>' : '') +
      '</td>' +
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
    const locBadge = '<span class="badge status-warn" data-tip="card.loc_saving">-' + c.loc_saving + ' LOC</span>';

    card.innerHTML =
      '<div class="item-card-head">' +
      '  <div style="display:flex; align-items:center; gap:0.4rem;">' +
      '    <span class="' + detBadgeClass + '" data-tip="det.' + c.detector.toLowerCase() + '">' + c.detector + '</span>' +
      '    <span class="badge" data-tip="card.overlap" style="background:var(--bg-subtle);">' + c.overlap_pct + '% overlap</span>' +
      '    ' + locBadge +
      '  </div>' +
      '  ' + repoBadge +
      '</div>' +
      '<div class="item-key">' + escapeHtml(c.symbol) + '</div>' +
      '<div class="item-snippet">&rarr; canonical: <code>' + escapeHtml(c.canonical) + '</code> <span class="badge status-ok" data-tip="card.callers">[callers=' + c.canonical_callers + ']</span></div>' +
      '<div class="item-meta">' +
      '  <span>' + escapeHtml(c.canonical_file) + ':' + c.canonical_line + '</span>' +
      (c.shared_evidence ? '<span>&bull;</span><span>' + escapeHtml(c.shared_evidence) + '</span>' : '') +
      '</div>';

    container.appendChild(card);
  });

  if (filtered[selectedReuseIndex]) {
    attachTipDescriptions(container);
    renderReuseDossier(filtered[selectedReuseIndex]);
  } else {
    selectedReuseIndex = 0;
    attachTipDescriptions(container);
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
    '      <span class="' + detBadgeClass + '" data-tip="det.' + c.detector.toLowerCase() + '">' + c.detector + '</span>' +
    '      <span class="badge status-ok" data-tip="card.overlap">' + c.overlap_pct + '% OVERLAP</span>' +
    '      <span class="badge badge-repo">' + escapeHtml(c.repo) + '</span>' +
    '    </div>' +
    '    <span class="badge status-warn" data-tip="card.loc_saving">Save ~' + c.loc_saving + ' LOC</span>' +
    '  </div>' +
    '  <div class="dossier-title">' + escapeHtml(c.symbol) + '</div>' +
    '  <div class="text-muted" style="font-size:0.82rem;">Authoritative Target: <strong>' + escapeHtml(c.canonical) + '</strong> <span class="badge status-ok">[callers=' + c.canonical_callers + ']</span></div>' +
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
    '</div>' +
    '<div class="action-recommendation">' +
    '  <strong>Consolidation Recommendation:</strong> Import authoritative helper <code>' + escapeHtml(c.canonical) + '</code> <span class="badge status-ok" data-tip="card.callers">[callers=' + c.canonical_callers + ']</span> and remove redundant definition <code>' + escapeHtml(c.symbol) + '</code> to eliminate drift and save ~' + c.loc_saving + ' LOC.' +
    '</div>';

  attachTipDescriptions(container);
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
    const rName = s.repo_name || 'workspace';
    tr.innerHTML =
      '<td class="num-col text-muted">' + ts + '</td>' +
      '<td><span class="repo-tag">' + escapeHtml(rName) + '</span></td>' +
      '<td><code>' + escapeHtml(s.tool) + '</code></td>' +
      '<td class="args-cell"><code>' + escapeHtml(s.terms || '(all paths)') + '</code></td>' +
      '<td class="text-right num-col">' + (s.shown || 0) + '</td>' +
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
    container.innerHTML = '<div class="empty-cell">No memory notes match the filter.<br><span style="font-size: 0.8rem; opacity: 0.7;">Agents write memories via <code>eg mem write --body "..." --kind gotcha &lt;key&gt;</code></span></div>';
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

    const kindClass = 'badge status-' + (m.kind === 'decision' ? 'ambig' : m.kind === 'gotcha' ? 'warn' : m.kind === 'fact' ? 'ok' : m.kind === 'procedure' ? 'neutral' : 'err');
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
    refsHtml = '<div class="dossier-section-title">Guarded Code References (.engram/index.db verified)</div><div class="chips-row">';
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
let isDocumentVisible = true;

// Selective Invalidation Debounce Timers
let summaryDebounceTimer = null;
let reuseDebounceTimer = null;
let memoriesDebounceTimer = null;

function triggerSummaryRefresh() {
  if (summaryDebounceTimer) clearTimeout(summaryDebounceTimer);
  summaryDebounceTimer = setTimeout(loadSummaryData, 3000);
}

function triggerReuseRefresh() {
  if (reuseDebounceTimer) clearTimeout(reuseDebounceTimer);
  reuseDebounceTimer = setTimeout(loadReuseData, 1000);
}

function triggerMemoriesRefresh() {
  if (memoriesDebounceTimer) clearTimeout(memoriesDebounceTimer);
  memoriesDebounceTimer = setTimeout(loadMemoriesData, 500);
}

function flashElement(el) {
  if (!el) return;
  el.classList.remove('metric-flash');
  void el.offsetWidth;
  el.classList.add('metric-flash');
}

function estimateEventTokens(tool, shown, resultCount) {
  switch (tool) {
    case 'context':
    case 'callers':
    case 'callees':
    case 'path':
    case 'impact':
    case 'imports':
      return shown * 35 + 800;
    case 'search':
      return shown * 25 + 800;
    case 'reuse':
      return resultCount * 350 + 1200;
    case 'symbols':
      return shown * 15;
    case 'build':
      return 1500;
    default:
      return 0;
  }
}

function patchSummaryState(q) {
  if (!latestSummaryData) return;

  const tool = q.tool || '';
  const repoName = q.repo_name || 'workspace';
  const agentName = q.agent || 'user';
  const exitCode = parseInt(q.exit_code || 0, 10);
  const isError = exitCode >= 3;
  const shown = q.shown || 0;
  const resultCount = q.result_count || 0;
  const isGraph = GRAPH_TOOLS.includes(tool);
  const isReuse = tool === 'reuse' || tool === 'search';
  const isMem = tool.startsWith('mem_');
  const isMemRead = tool === 'mem_query' || tool === 'mem_get' || tool === 'mem_export';
  const isMemWrite = tool === 'mem_write' || tool === 'mem_delete' || tool === 'mem_touch' || tool === 'mem_gc';

  const savedTokens = estimateEventTokens(tool, shown, resultCount);

  // 1. Overall & Graph Totals
  latestSummaryData.total_calls = (latestSummaryData.total_calls || 0) + 1;
  if (isError) {
    latestSummaryData.error_count = (latestSummaryData.error_count || 0) + 1;
  }
  const tot = latestSummaryData.total_calls;
  const err = latestSummaryData.error_count || 0;
  latestSummaryData.reliability_pct = tot > 0 ? (100.0 * (tot - err) / tot) : 100.0;
  latestSummaryData.saved_tokens = (latestSummaryData.saved_tokens || 0) + savedTokens;
  latestSummaryData.saved_dollars = (latestSummaryData.saved_tokens / 1000000.0) * 3.0;

  if (isGraph) {
    latestSummaryData.graph_calls = (latestSummaryData.graph_calls || 0) + 1;
    if (isError) {
      latestSummaryData.graph_errors = (latestSummaryData.graph_errors || 0) + 1;
    }
    const gTot = latestSummaryData.graph_calls;
    const gErr = latestSummaryData.graph_errors || 0;
    latestSummaryData.graph_reliability_pct = gTot > 0 ? (100.0 * (gTot - gErr) / gTot) : 100.0;
    latestSummaryData.graph_saved_tokens = (latestSummaryData.graph_saved_tokens || 0) + savedTokens;
    latestSummaryData.graph_saved_dollars = (latestSummaryData.graph_saved_tokens / 1000000.0) * 3.0;
  }

  if (tool === 'reuse' && exitCode === 0) {
    const hits = Math.max(shown, resultCount);
    if (hits > 0) {
      latestSummaryData.reuse_hits = (latestSummaryData.reuse_hits || 0) + hits;
    }
  }

  // 2. Tool Aggs
  if (!latestSummaryData.tool_aggs) latestSummaryData.tool_aggs = {};
  if (!latestSummaryData.tool_aggs[tool]) {
    latestSummaryData.tool_aggs[tool] = { calls: 0, errors: 0, shown: 0 };
  }
  const tEntry = latestSummaryData.tool_aggs[tool];
  tEntry.calls = (tEntry.calls || 0) + 1;
  tEntry.shown = (tEntry.shown || 0) + shown;
  if (isError) tEntry.errors = (tEntry.errors || 0) + 1;

  // 3. Repo Aggs
  if (!latestSummaryData.repo_aggs) latestSummaryData.repo_aggs = {};
  if (!latestSummaryData.repo_aggs[repoName]) {
    latestSummaryData.repo_aggs[repoName] = {
      calls: 0, errors: 0, graph_calls: 0, graph_errors: 0,
      saved_tokens: 0, graph_saved_tokens: 0,
      reuse_calls: 0, reuse_errors: 0, reuse_hits: 0, reuse_saved_tokens: 0,
      memory_calls: 0, memory_errors: 0, memory_reads: 0, memory_writes: 0,
      worktree: Boolean(q.worktree),
    };
  }
  const rEntry = latestSummaryData.repo_aggs[repoName];
  rEntry.calls = (rEntry.calls || 0) + 1;
  if (isError) rEntry.errors = (rEntry.errors || 0) + 1;
  rEntry.saved_tokens = (rEntry.saved_tokens || 0) + savedTokens;
  rEntry.worktree = rEntry.worktree || Boolean(q.worktree);
  if (q.ts) rEntry.last_ts = q.ts;
  if (isGraph) {
    rEntry.graph_calls = (rEntry.graph_calls || 0) + 1;
    if (isError) rEntry.graph_errors = (rEntry.graph_errors || 0) + 1;
    rEntry.graph_saved_tokens = (rEntry.graph_saved_tokens || 0) + savedTokens;
    rEntry.graph_reliability_pct = rEntry.graph_calls ? 100.0 * (rEntry.graph_calls - rEntry.graph_errors) / rEntry.graph_calls : 100.0;
  }
  if (isReuse) {
    rEntry.reuse_calls = (rEntry.reuse_calls || 0) + 1;
    if (isError) rEntry.reuse_errors = (rEntry.reuse_errors || 0) + 1;
    rEntry.reuse_saved_tokens = (rEntry.reuse_saved_tokens || 0) + savedTokens;
    if (tool === 'reuse' && exitCode === 0) {
      const hits = Math.max(shown, resultCount);
      rEntry.reuse_hits = (rEntry.reuse_hits || 0) + hits;
    }
    rEntry.reuse_reliability_pct = rEntry.reuse_calls ? 100.0 * (rEntry.reuse_calls - rEntry.reuse_errors) / rEntry.reuse_calls : 100.0;
  }
  if (isMem) {
    rEntry.memory_calls = (rEntry.memory_calls || 0) + 1;
    if (isError) rEntry.memory_errors = (rEntry.memory_errors || 0) + 1;
    if (isMemRead) rEntry.memory_reads = (rEntry.memory_reads || 0) + 1;
    if (isMemWrite) rEntry.memory_writes = (rEntry.memory_writes || 0) + 1;
    rEntry.memory_reliability_pct = rEntry.memory_calls ? 100.0 * (rEntry.memory_calls - rEntry.memory_errors) / rEntry.memory_calls : 100.0;
  }

  // 4. Agent Aggs
  if (!latestSummaryData.agent_aggs) latestSummaryData.agent_aggs = {};
  if (!latestSummaryData.agent_aggs[agentName]) {
    latestSummaryData.agent_aggs[agentName] = {
      calls: 0, errors: 0, graph_calls: 0, graph_errors: 0,
      saved_tokens: 0, graph_saved_tokens: 0,
      reuse_calls: 0, reuse_errors: 0, reuse_hits: 0, reuse_saved_tokens: 0,
      memory_calls: 0, memory_errors: 0, memory_reads: 0, memory_writes: 0,
    };
  }
  const aEntry = latestSummaryData.agent_aggs[agentName];
  aEntry.calls = (aEntry.calls || 0) + 1;
  if (isError) aEntry.errors = (aEntry.errors || 0) + 1;
  aEntry.saved_tokens = (aEntry.saved_tokens || 0) + savedTokens;
  if (q.ts) aEntry.last_ts = q.ts;
  if (isGraph) {
    aEntry.graph_calls = (aEntry.graph_calls || 0) + 1;
    if (isError) aEntry.graph_errors = (aEntry.graph_errors || 0) + 1;
    aEntry.graph_saved_tokens = (aEntry.graph_saved_tokens || 0) + savedTokens;
    aEntry.graph_reliability_pct = aEntry.graph_calls ? 100.0 * (aEntry.graph_calls - aEntry.graph_errors) / aEntry.graph_calls : 100.0;
  }
  if (isReuse) {
    aEntry.reuse_calls = (aEntry.reuse_calls || 0) + 1;
    if (isError) aEntry.reuse_errors = (aEntry.reuse_errors || 0) + 1;
    aEntry.reuse_saved_tokens = (aEntry.reuse_saved_tokens || 0) + savedTokens;
    if (tool === 'reuse' && exitCode === 0) {
      const hits = Math.max(shown, resultCount);
      aEntry.reuse_hits = (aEntry.reuse_hits || 0) + hits;
    }
    aEntry.reuse_reliability_pct = aEntry.reuse_calls ? 100.0 * (aEntry.reuse_calls - aEntry.reuse_errors) / aEntry.reuse_calls : 100.0;
  }
  if (isMem) {
    aEntry.memory_calls = (aEntry.memory_calls || 0) + 1;
    if (isError) aEntry.memory_errors = (aEntry.memory_errors || 0) + 1;
    if (isMemRead) aEntry.memory_reads = (aEntry.memory_reads || 0) + 1;
    if (isMemWrite) aEntry.memory_writes = (aEntry.memory_writes || 0) + 1;
    aEntry.memory_reliability_pct = aEntry.memory_calls ? 100.0 * (aEntry.memory_calls - aEntry.memory_errors) / aEntry.memory_calls : 100.0;
  }
}

// Live Server-Sent Events (SSE)
function initEventSource() {
  const liveBadge = document.getElementById('live-indicator');
  if (liveBadge) liveBadge.style.display = 'inline-block';

  const source = new EventSource('/events');

  source.onopen = function() {
    const dot = document.querySelector('.live-dot');
    if (dot) {
      dot.className = 'live-dot live-active';
    }
    if (liveBadge) liveBadge.title = 'Live stream connected';
  };

  source.onerror = function() {
    const dot = document.querySelector('.live-dot');
    if (dot) {
      dot.className = 'live-dot live-reconnecting';
    }
    if (liveBadge) liveBadge.title = 'Reconnecting to live stream...';
  };

  source.onmessage = function(e) {
    try {
      const q = JSON.parse(e.data);
      handleLiveEvent(q);
      liveEventCount++;

      // Lazy background drift reconciliation (debounced 3s)
      triggerSummaryRefresh();

      // Mutation-selective endpoint invalidation
      const tool = q.tool || '';
      if (tool === 'build') {
        triggerReuseRefresh();
      } else if (tool === 'mem_write' || tool === 'mem_delete' || tool === 'mem_touch') {
        triggerMemoriesRefresh();
      }
    } catch(err) {}
  };

  // Page Visibility API to save battery and sync on foreground
  document.addEventListener('visibilitychange', () => {
    isDocumentVisible = !document.hidden;
    const dot = document.querySelector('.live-dot');
    if (isDocumentVisible) {
      if (dot) dot.className = 'live-dot live-active';
      loadSummaryData();
      if (activeTab === 'reuse') loadReuseData();
      if (activeTab === 'memory') loadMemoriesData();
    } else {
      if (dot) dot.className = 'live-dot live-paused';
    }
  });

  // Background fallback poll every 15s to keep metrics fresh when visible
  setInterval(() => {
    if (!isDocumentVisible) return;
    loadSummaryData();
    if (activeTab === 'reuse') loadReuseData();
    if (activeTab === 'memory') loadMemoriesData();
  }, 15000);
}

function handleLiveEvent(q) {
  const tool = q.tool || '';
  const isGraph = GRAPH_TOOLS.includes(tool);
  const exitCode = parseInt(q.exit_code || 0, 10);

  // 1. Immediately patch in-memory aggregation state
  patchSummaryState(q);

  // 2. Real-time updates for Tab 01 (Graph Navigation)
  if (isGraph) {
    totalCalls = latestSummaryData ? latestSummaryData.graph_calls : (totalCalls + 1);
    totalErrors = latestSummaryData ? latestSummaryData.graph_errors : (exitCode >= 3 ? totalErrors + 1 : totalErrors);

    const totalCallsEl = document.getElementById('metric-total-calls');
    if (totalCallsEl) {
      totalCallsEl.textContent = totalCalls.toLocaleString();
      flashElement(totalCallsEl);
    }

    const relEl = document.getElementById('metric-reliability');
    if (relEl) {
      const rel = latestSummaryData ? latestSummaryData.graph_reliability_pct : (totalCalls > 0 ? (100.0 * (totalCalls - totalErrors) / totalCalls) : 100.0);
      relEl.textContent = rel.toFixed(1) + "% clean (" + totalErrors + " errs)";
    }

    const tokEl = document.getElementById('metric-tokens');
    if (tokEl && latestSummaryData) {
      tokEl.textContent = '~' + (latestSummaryData.graph_saved_tokens || 0).toLocaleString();
    }

    const dolEl = document.getElementById('metric-dollars');
    if (dolEl && latestSummaryData) {
      dolEl.textContent = '~$' + (latestSummaryData.graph_saved_dollars || 0.0).toFixed(2) + ' avoided vs full reads';
    }

    const recentTbody = document.getElementById('recent-tbody');
    if (recentTbody) {
      const emptyRow = recentTbody.querySelector('.empty-cell');
      if (emptyRow) recentTbody.innerHTML = '';
      const row = createRecentRow(q);
      recentTbody.insertBefore(row, recentTbody.firstChild);
      while (recentTbody.children.length > 50) {
        recentTbody.removeChild(recentTbody.lastChild);
      }
    }
  } else if (tool === 'search' || tool === 'reuse') {
    // 3. Real-time updates for Tab 02 (Anti-Reinvention)
    const args = q.args || {};
    const terms = args.terms || args.paths || '';
    const newSearch = {
      ts: q.ts,
      repo_name: q.repo_name,
      tool: tool,
      terms: terms,
      shown: q.shown || q.result_count || 0,
      latency_us: q.latency_us || 0,
    };
    prewriteSearches.unshift(newSearch);
    if (prewriteSearches.length > 40) prewriteSearches.pop();
    renderPrewriteTable();
    const feedCountEl = document.getElementById('reuse-feed-count');
    if (feedCountEl) {
      const cur = parseInt(feedCountEl.textContent.replace(/,/g, ''), 10) || 0;
      feedCountEl.textContent = (cur + 1).toLocaleString();
      flashElement(feedCountEl);
    }
  } else if (tool.startsWith('mem_')) {
    updateMemoryKpis();
    const currencyEl = document.getElementById('mem-kpi-currency');
    if (currencyEl) flashElement(currencyEl);
  }

  // 5. Update the active tab breakdown tables immediately using in-memory state
  if (latestSummaryData) {
    if (activeTab === 'telemetry') {
      renderReposTable(latestSummaryData.repo_aggs || {});
      renderAgentsTable(latestSummaryData.agent_aggs || {}, latestSummaryData.graph_calls || latestSummaryData.total_calls);
      renderToolsTable(latestSummaryData.tool_aggs || {}, latestSummaryData.graph_calls || latestSummaryData.total_calls);
    } else if (activeTab === 'reuse') {
      renderReuseReposTable(latestSummaryData.repo_aggs || {});
      renderReuseAgentsTable(latestSummaryData.agent_aggs || {});
    } else if (activeTab === 'memory') {
      renderMemoryReposTable(latestSummaryData.repo_aggs || {});
      renderMemoryAgentsTable(latestSummaryData.agent_aggs || {});
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

// Hover help. One copy table, one shared node, delegated listeners.
const TIP = {
  'kpi.graph_queries': 'Code lookups agents ran against the index instead of reading files.',
  'kpi.p50': 'Half of all queries finished faster than this.',
  'kpi.tokens_saved': 'Tokens an agent did not spend reading source files.',
  'kpi.scope': 'Repositories and worktrees with an index in this log.',
  'kpi.candidates': 'Functions that look like near-copies of another function.',
  'kpi.excess_loc': 'Lines you could delete by merging duplicates. Estimated from sampled candidates.',
  'kpi.recall': 'Share of tagged sessions that read memory before acting.',
  'kpi.currency': 'Share of memory refs that still point at existing code.',
  'col.last_active': 'Most recent run recorded for this row.',
  'col.p50': 'Half of runs were faster than this.',
  'col.items': 'Results returned by the query.',
  'col.reliability': 'Share of runs that finished without error.',
  'col.tokens_saved': 'Tokens saved versus reading the source files.',
  'col.hits': 'Duplicate candidates found in those runs.',
  'col.reads_writes': 'Memory reads versus notes written.',
  'card.overlap': 'How much evidence the two functions share.',
  'card.callers': 'Places that already call the recommended helper.',
  'card.loc_saving': 'Lines removed if the duplicate is merged away.',
  'det.all': 'Show every detection rule.',
  'det.d1': 'Both functions call the same downstream functions.',
  'det.d2': 'Same parameter types and name tokens.',
  'det.d3': 'Calls a raw client the project already wraps.',
  'det.d4': 'One-caller wrapper that only forwards a call.',
  'filter.status': 'Show only runs with these exit codes.',
  'filter.superseded': 'Show older revisions replaced by a newer note.'
};

let tipSeq = 0;

function attachTipDescriptions(root) {
  const scope = root || document;
  scope.querySelectorAll('[data-tip]').forEach((el) => {
    const text = TIP[el.getAttribute('data-tip')];
    if (!text || el.querySelector('.sr-only')) return;
    tipSeq += 1;
    const id = 'tip-' + tipSeq;
    const span = document.createElement('span');
    span.className = 'sr-only';
    span.id = id;
    span.textContent = text;
    el.appendChild(span);
    el.setAttribute('aria-describedby', id);
  });
}

function initTips() {
  const tip = document.createElement('div');
  tip.className = 'eg-tip';
  tip.id = 'eg-tip';
  tip.setAttribute('role', 'tooltip');
  document.body.appendChild(tip);

  // Static hosts get their description up front so a screen reader reads it on
  // focus, before any hover happens. Re-rendered content calls this again.
  attachTipDescriptions(document);

  let timer = null;
  let current = null;

  const place = (el) => {
    const r = el.getBoundingClientRect();
    const t = tip.getBoundingClientRect();
    let top = r.top - t.height - 8;
    if (top < 8) top = r.bottom + 8;
    let left = r.left + r.width / 2 - t.width / 2;
    left = Math.max(8, Math.min(left, window.innerWidth - t.width - 8));
    tip.style.top = Math.round(top) + 'px';
    tip.style.left = Math.round(left) + 'px';
  };

  const show = (el) => {
    const text = TIP[el.getAttribute('data-tip')];
    if (!text) return;
    current = el;
    tip.textContent = text;
    tip.classList.add('visible');
    place(el);
  };

  const hide = () => {
    clearTimeout(timer);
    current = null;
    tip.classList.remove('visible');
  };

  document.addEventListener('mouseover', (e) => {
    if (window.matchMedia('(pointer: coarse)').matches) return;
    const el = e.target.closest('[data-tip]');
    if (!el || el === current) return;
    clearTimeout(timer);
    timer = setTimeout(() => show(el), 250);
  });

  document.addEventListener('mouseout', (e) => {
    const el = e.target.closest('[data-tip]');
    if (!el) return;
    if (el === current) hide();
    else clearTimeout(timer);
  });

  document.addEventListener('focusin', (e) => {
    const el = e.target.closest('[data-tip]');
    if (el) show(el);
  });
  document.addEventListener('focusout', hide);
  document.addEventListener('keydown', (e) => {
    if (e.key === 'Escape') hide();
  });
  window.addEventListener('scroll', hide, true);
}

window.addEventListener('DOMContentLoaded', () => {
  initTheme();
  initTips();
  loadSummaryData();
  loadReuseData();
  loadMemoriesData();
  initEventSource();
});
