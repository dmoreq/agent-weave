// eg Native Rust Dashboard Client (Zero External Dependencies)

const GRAPH_TOOLS = ['callers', 'callees', 'path', 'context', 'impact', 'symbols', 'imports', 'build'];
const FEED_LIMIT = 10; // both live feeds show the newest ten rows only
const UNATTRIBUTED = '(unattributed)';

// One builder for every agent tag so the copy cannot drift between tables.
// `user` is the fallback when no agent marker was detected, so its tag has to
// say what it does not know instead of claiming a human typed the command.
// Evidence label for detectors that carry no measured overlap (D3/D4).
function detectorEvidenceLabel(detector) {
  if (detector === 'D3') return 'shared guard';
  if (detector === 'D4') return 'pass-through wrapper';
  if (detector === 'D5') return 'class shape';
  if (detector === 'D1L') return 'unresolved call';
  if (detector === 'DT') return 'test overlap';
  return 'evidence';
}

function agentTagHtml(aName) {
  const title = aName === 'user'
    ? ' title="No agent marker detected — a human terminal or an unidentified invoker (ENGRAM_AGENT=<name> labels it precisely)"'
    : '';
  return '<span class="agent-tag agent-tag-' + escapeHtml(aName) + '"' + title + '>' + escapeHtml(aName) + '</span>';
}

let allMemories = [];
let recentQueries = [];
let reuseCandidates = [];
let prewriteSearches = [];
let totalCalls = 0;
let totalErrors = 0;
let latestSummaryData = null;

let selectedMemoryId = null;   // store_id:key:rev — survives list rebuilds
let selectedReuseId = null;    // candidate id (repo:symbol->canonical)
function memorySelectionId(m) { return (m.store_id || m.store) + ':' + m.key + ':' + m.rev; }

// Collapsed list views. "Recent" means latest activity: created, verified, or
// recalled. Reuse has no per-candidate timestamp in the payload, so its
// ranking is call-site usage only; nothing here claims recency it cannot know.
const MEMORY_VIEWS = {
  recent10: { label: 'Top 10 recent', limit: 10, sort: 'activity' },
  recent3: { label: 'Top 3 recent', limit: 3, sort: 'activity' },
  used: { label: 'Most used', limit: 10, sort: 'used' },
  all: { label: 'All', limit: 0, sort: 'activity' },
};
const REUSE_VIEWS = {
  used10: { label: 'Top 10 by callers', limit: 10 },
  used3: { label: 'Top 3 by callers', limit: 3 },
  all: { label: 'All', limit: 0 },
};
let memView = 'recent3';
let reuseView = 'used3';

function memoryActivity(m) {
  return Math.max(m.created_at || 0, m.anchored_at || 0, m.last_recalled_ts || 0);
}

function viewMemories(items) {
  const spec = MEMORY_VIEWS[memView] || MEMORY_VIEWS.recent3;
  const sorted = items.slice().sort((a, b) => {
    if (spec.sort === 'used') {
      const used = (b.recall_count || 0) - (a.recall_count || 0);
      if (used !== 0) return used;
    }
    const activity = memoryActivity(b) - memoryActivity(a);
    if (activity !== 0) return activity;
    const recalled = (b.last_recalled_ts || 0) - (a.last_recalled_ts || 0);
    if (recalled !== 0) return recalled;
    const created = (b.created_at || 0) - (a.created_at || 0);
    if (created !== 0) return created;
    return String(a.key).localeCompare(String(b.key));
  });
  const visible = spec.limit > 0 ? sorted.slice(0, spec.limit) : sorted;
  return { items: visible, total: items.length, label: spec.label };
}

function viewReuseCandidates(items) {
  const spec = REUSE_VIEWS[reuseView] || REUSE_VIEWS.used3;
  const sorted = items.slice().sort((a, b) => {
    const callers = (b.canonical_callers || 0) - (a.canonical_callers || 0);
    if (callers !== 0) return callers;
    const ap = a.overlap_applicable ? (a.overlap_pct || 0) : -1;
    const bp = b.overlap_applicable ? (b.overlap_pct || 0) : -1;
    if (bp !== ap) return bp - ap;
    return String(a.id).localeCompare(String(b.id));
  });
  const visible = spec.limit > 0 ? sorted.slice(0, spec.limit) : sorted;
  return { items: visible, total: items.length, label: spec.label };
}

// Reuse "last used" comes from the retained journal: an `eg reuse` run that
// surfaced this candidate. Zero means no run in the retained window.
function reuseUsageText(c) {
  if (!c.reuse_count) return 'never surfaced in retained journal';
  const when = c.last_reused_ts
    ? new Date(c.last_reused_ts * 1000).toLocaleString([], { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' })
    : '—';
  return 'surfaced ' + c.reuse_count + '\u00d7 \u00b7 last used ' + when;
}

function updateListCount(id, shown, total, label) {
  const el = document.getElementById(id);
  if (el) el.textContent = 'Showing ' + shown + ' of ' + total + ' · ' + label;
}
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

// Tab Switching. The allowlist is the guard: an unknown key would otherwise
// hide every panel and leave the console blank.
const TAB_KEYS = ['telemetry', 'reuse', 'memory'];
function switchTab(tab) {
  if (!TAB_KEYS.includes(tab)) {
    console.warn('switchTab: unknown tab "' + tab + '" — falling back to telemetry');
    tab = 'telemetry';
  }
  activeTab = tab;
  const tabs = TAB_KEYS;
  tabs.forEach(t => {
    const btn = document.getElementById('tab-' + t + '-btn');
    const content = document.getElementById('tab-' + t + '-content');
    const selected = t === tab;
    if (btn) {
      btn.classList.toggle('active', selected);
      btn.setAttribute('aria-selected', selected ? 'true' : 'false');
      btn.tabIndex = selected ? 0 : -1;
    }
    if (content) content.classList.toggle('hidden', !selected);
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
    .then(r => {
      if (!r.ok) throw new Error('HTTP ' + r.status);
      return r.json();
    })
    .then(data => {
      clearFetchError('telemetry');
      setDataAge(data.generated_at);
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
      // Render Recent Queries (Graph only); re-apply active filters so a
      // refetch cannot silently widen the view.
      renderRecentQueries(data.graph_recent_queries || []);
      applyTelemetryFilters();

      // Update Memory KPIs
      updateMemoryKpis();
    })
    .catch(err => showFetchError('telemetry', err.message));
}

function loadReuseData() {
  fetch('/api/reuse')
    .then(r => {
      if (!r.ok) throw new Error('HTTP ' + r.status);
      return r.json();
    })
    .then(data => {
      clearFetchError('reuse');
      reuseCandidates = data.candidates || [];
      prewriteSearches = data.prewrite_searches || [];

      const summary = data.summary || {};
      const totalCandidates = summary.total_candidates !== undefined ? summary.total_candidates : reuseCandidates.length;

      const kpiCount = document.getElementById('reuse-kpi-count');
      if (kpiCount) kpiCount.textContent = totalCandidates.toLocaleString();

      const kpiCountSub = document.getElementById('reuse-kpi-count-sub');
      if (kpiCountSub) {
        const cacheNote = summary.cache_hit
          ? ' · cached ' + Math.round((summary.cache_age_ms || 0) / 1000) + 's'
          : '';
        if (totalCandidates > reuseCandidates.length) {
          kpiCountSub.textContent = 'Top ' + reuseCandidates.length + ' of ' + totalCandidates.toLocaleString() + ' duplicate twins' + cacheNote;
        } else {
          kpiCountSub.textContent = 'Syntactic & structural duplicate twins' + cacheNote;
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
    .catch(err => showFetchError('reuse', err.message));
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
    .then(r => {
      if (!r.ok) throw new Error('HTTP ' + r.status);
      return r.json();
    })
    .then(data => {
      clearFetchError('memory');
      if (data.degraded) {
        showFetchError('memory', (data.errors || []).map(e => e.store + ': ' + e.message).join('; '));
      }
      allMemories = data.notes || [];
      updateMemoryKpis();
      populateMemoryStoreFilter();
      renderMemoryList();
    })
    .catch(err => showFetchError('memory', err.message));
}

function updateMemoryKpis() {
  const currentNotes = allMemories.filter(m => m.is_current);

  // KPI 1: Session Recall Rate
  // The rate only means something when callers tag their sessions: without a
  // tag every run collapses into the synthetic `default` bucket.
  const taggedSessions = latestSummaryData ? (latestSummaryData.session_count || 0) : 0;
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
      recallSubEl.textContent = memSessions + ' of ' + totalSessions + ' sessions'
        + (totalSessions < 5 ? ' — low sample; set ENGRAM_SESSION on agent runs' : '');
    }
  }

  // KPI 2: Anchor Currency Rate
  const notesWithRefs = currentNotes.filter(m => m.refs && m.refs.length > 0);
  let validCount = 0;
  let modifiedCount = 0;
  let brokenCount = 0;
  let unverifiedCount = 0;
  notesWithRefs.forEach(m => {
    if (m.ref_status === 'valid') validCount++;
    else if (m.ref_status === 'broken') brokenCount++;
    else if (m.ref_status === 'modified') modifiedCount++;
    else if (m.ref_status === 'unverified') unverifiedCount++;
  });
  // No anchored note means the ratio is unmeasurable, not 100%.
  const currencyRate = notesWithRefs.length > 0
    ? (100.0 * validCount / notesWithRefs.length).toFixed(1) + '%'
    : '—';
  const currencyEl = document.getElementById('mem-kpi-currency');
  if (currencyEl) currencyEl.textContent = currencyRate;
  const currencySubEl = document.getElementById('mem-kpi-currency-sub');
  if (currencySubEl) {
    currencySubEl.textContent = notesWithRefs.length === 0
      ? 'No notes with code refs'
      : validCount + ' valid · ' + modifiedCount + ' modified · ' + brokenCount + ' broken'
        + (unverifiedCount > 0 ? ' · ' + unverifiedCount + ' unverified' : '');
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
    const verifyAll = modifiedCount > 0
      ? '<button class="btn-touch" id="verify-all-btn" style="margin-left:0.5rem;">Verify all ' + modifiedCount + '</button>'
      : '';
    summaryEl.innerHTML = modifiedCount + ' modified · ' + brokenCount + ' broken requiring attention' + verifyAll;
  }

  itemsEl.innerHTML = '';
  actionable.forEach(m => {
    const isModified = m.ref_status === 'modified';
    const badge = isModified
      ? '<span class="badge status-warn">[MODIFIED REF]</span>'
      : '<span class="badge status-err">[BROKEN REF]</span>';
    const explanation = isModified
      ? 'Target file was modified after the last verify. Review the note, then Verify to refresh its anchor.'
      : 'Referenced symbol or file path is missing from the current code index. Re-run <code>eg build</code> in that repository, or edit the note refs.';
    const touchBtn = isModified
      ? '<button class="btn-touch" data-key="' + escapeHtml(m.key) + '" data-store-id="' + escapeHtml(m.store_id || m.store) + '" data-rev="' + m.rev + '">Verify</button>'
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

function touchMemory(btn) {
  const key = btn.dataset.key;
  const storeId = btn.dataset.storeId;
  const rev = parseInt(btn.dataset.rev || '0', 10);
  fetch('/api/memory/touch', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ key: key, store_id: storeId, rev: rev })
  })
    .then(r => r.json().then(body => ({ status: r.status, body: body })))
    .then(res => {
      if (res.body && res.body.ok) {
        btn.disabled = true;
        btn.textContent = 'Verified';
        loadMemoriesData();
        return;
      }
      const msg = (res.body && res.body.error) || ('HTTP ' + res.status);
      showTouchError(btn, msg);
    })
    .catch(err => showTouchError(btn, 'network error: ' + err.message));
}

function showTouchError(btn, message) {
  let el = btn.parentElement ? btn.parentElement.querySelector('.touch-error') : null;
  if (!el) {
    el = document.createElement('span');
    el.className = 'touch-error text-muted';
    el.style.fontSize = '0.72rem';
    el.style.marginLeft = '0.5rem';
    if (btn.parentElement) btn.parentElement.appendChild(el);
  }
  el.textContent = message;
}

function verifyAllModified() {
  const currentNotes = allMemories.filter(m => m.is_current && m.ref_status === 'modified');
  if (currentNotes.length === 0) return;
  if (!confirm('Verify all ' + currentNotes.length + ' modified note(s)? This refreshes their anchor timestamps.')) return;
  let pending = currentNotes.length;
  let failed = 0;
  currentNotes.forEach(m => {
    fetch('/api/memory/touch', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ key: m.key, store_id: m.store_id || m.store, rev: m.rev })
    })
      .then(r => r.json())
      .then(res => { if (!res.ok) failed++; })
      .catch(() => { failed++; })
      .finally(() => {
        pending--;
        if (pending === 0) {
          announce(failed === 0 ? 'Verified ' + currentNotes.length + ' notes' : failed + ' of ' + currentNotes.length + ' verifies failed');
          loadMemoriesData();
        }
      });
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
    // The dropdown matches recent-row `data-repo` labels (the repo name), so
    // value and text must both be the display label, never the path key.
    const label = rData.display || rName;
    if (select) {
      const opt = document.createElement('option');
      opt.value = label;
      opt.textContent = label;
      select.appendChild(opt);
    }

    const worktrees = Object.entries(rData.worktrees || {});
    const expandable = worktrees.length > 0;

    const tr = document.createElement('tr');
    tr.className = 'repo-row-item';
    tr.tabIndex = 0;
    tr.setAttribute('role', 'row');
    if (expandable) {
      tr.setAttribute('aria-label', 'Toggle worktrees for repository ' + label);
      tr.setAttribute('aria-expanded', 'false');
    } else {
      tr.setAttribute('aria-label', 'Filter by repository ' + label);
    }

    const calls = rData.calls;
    const errs = rData.errors || 0;
    const rel = rData.reliability_pct;
    const wt = rData.worktree;
    const lastStr = rData.last_ts ? new Date(rData.last_ts * 1000).toLocaleString() : '—';
    const pct = (calls / maxCalls) * 100.0;
    const wtBadge = wt ? '<span class="badge badge-wt">worktree</span>' : '<span class="badge badge-repo">repo</span>';
    const relClass = errs === 0 ? 'status-ok' : 'status-err';
    const chevron = expandable
      ? '<span class="wt-chevron" aria-hidden="true"></span>'
      : '';
    const wtCount = expandable
      ? '<span class="text-muted wt-count">' + worktrees.length + (worktrees.length === 1 ? ' worktree' : ' worktrees') + '</span>'
      : '';

    tr.innerHTML =
      '<td>' +
      '  <div class="repo-name-wrap">' + chevron + '<strong>' + escapeHtml(label) + '</strong> ' + wtBadge + wtCount + '</div>' +
      '  <div class="row-track"><div class="row-track-fill" style="width: ' + pct.toFixed(1) + '%;"></div></div>' +
      '</td>' +
      '<td class="text-right num-col">' + calls.toLocaleString() + '</td>' +
      '<td><span class="badge ' + relClass + '">' + rel.toFixed(1) + '% (' + errs + ' err)</span></td>' +
      '<td class="text-muted num-col">' + lastStr + '</td>';

    tbody.appendChild(tr);

    // Linked-worktree sub-rows stay collapsed until the main row is clicked;
    // the repository filter dropdown still scopes the feed to the main repo.
    const subRows = worktrees.map(([wtName, wtData]) => {
      const wtCalls = wtData.calls || 0;
      const wtErrs = wtData.errors || 0;
      const wtRel = wtCalls ? 100.0 * (wtCalls - wtErrs) / wtCalls : 100.0;
      const wtLast = wtData.last_ts ? new Date(wtData.last_ts * 1000).toLocaleString() : '—';
      const sub = document.createElement('tr');
      sub.className = 'repo-worktree-row';
      sub.setAttribute('data-parent', label);
      sub.innerHTML =
        '<td><div class="repo-name-wrap wt-sub-name"><span class="badge badge-wt">worktree</span> ' + escapeHtml(wtName) + '</div></td>' +
        '<td class="text-right num-col">' + wtCalls.toLocaleString() + '</td>' +
        '<td><span class="badge ' + (wtErrs === 0 ? 'status-ok' : 'status-err') + '">' + wtRel.toFixed(1) + '% (' + wtErrs + ' err)</span></td>' +
        '<td class="text-muted num-col">' + wtLast + '</td>';
      tbody.appendChild(sub);
      return sub;
    });

    if (expandable) {
      const toggle = () => {
        const open = tr.getAttribute('aria-expanded') !== 'true';
        tr.setAttribute('aria-expanded', String(open));
        subRows.forEach(sub => sub.classList.toggle('wt-open', open));
        announce((open ? 'Expanded ' : 'Collapsed ') + worktrees.length + ' worktrees of ' + label);
      };
      tr.onclick = toggle;
      tr.onkeydown = (ev) => { if (ev.key === 'Enter' || ev.key === ' ') { ev.preventDefault(); toggle(); } };
    } else {
      tr.onclick = () => filterByRepo(label);
      tr.onkeydown = (ev) => { if (ev.key === 'Enter' || ev.key === ' ') { ev.preventDefault(); filterByRepo(label); } };
    }
  });
  if (select) {
    if (curRepoVal && Array.from(select.options).some(o => o.value === curRepoVal)) {
      select.value = curRepoVal;
    } else {
      select.value = 'ALL';
      if (curRepoVal && curRepoVal !== 'ALL') announce('Filter "' + curRepoVal + '" reset: repository no longer active');
    }
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
    Object.keys(agentAggs).sort().forEach(aName => {
      const opt = document.createElement('option');
      opt.value = aName;
      opt.textContent = aName;
      select.appendChild(opt);
    });
  }

  entries.forEach(([aName, aData]) => {

    const tr = document.createElement('tr');
    tr.className = 'agent-row-item';
    tr.tabIndex = 0;
    tr.setAttribute('role', 'row');
    tr.setAttribute('aria-label', 'Filter by agent ' + aName);
    tr.onclick = () => filterByAgent(aName);
    tr.onkeydown = (ev) => { if (ev.key === 'Enter' || ev.key === ' ') { ev.preventDefault(); filterByAgent(aName); } };

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
      '  <div class="repo-name-wrap">' + agentTagHtml(aName) + ' <span class="text-muted" style="font-size: 0.75rem;">(' + shareStr + ')</span></div>' +
      '  <div class="row-track"><div class="row-track-fill row-track-fill-agent" style="width: ' + pct.toFixed(1) + '%;"></div></div>' +
      '</td>' +
      '<td class="text-right num-col">' + calls.toLocaleString() + '</td>' +
      '<td><span class="badge ' + relClass + '">' + rel.toFixed(1) + '% (' + errs + ' err)</span></td>' +
      '<td class="text-right num-col text-muted">~' + saved.toLocaleString() + '</td>';

    tbody.appendChild(tr);
  });
  if (select) {
    if (curAgentVal && Array.from(select.options).some(o => o.value === curAgentVal)) {
      select.value = curAgentVal;
    } else {
      select.value = 'ALL';
      if (curAgentVal && curAgentVal !== 'ALL') announce('Filter "' + curAgentVal + '" reset: agent no longer active');
    }
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
    tr.tabIndex = 0;
    tr.setAttribute('role', 'row');
    tr.setAttribute('aria-label', 'Filter by repository ' + (rData.display || rName));
    // Candidate filters match store basenames; the path key would append a
    // ghost option and empty the list.
    tr.onclick = () => filterReuseByRepo(rData.display || rName);
    tr.onkeydown = (ev) => { if (ev.key === 'Enter' || ev.key === ' ') { ev.preventDefault(); filterReuseByRepo(rData.display || rName); } };

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
      switchTab('telemetry');
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
      '  <div class="repo-name-wrap">' + agentTagHtml(aName) + ' <span class="text-muted" style="font-size: 0.75rem;">(' + shareStr + ')</span></div>' +
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
    tr.tabIndex = 0;
    tr.setAttribute('role', 'row');
    tr.setAttribute('aria-label', 'Filter by store ' + (rData.display || rName));
    // Store filters match store basenames; the path key would append a ghost
    // option and empty the list.
    tr.onclick = () => filterMemoryByRepo(rData.display || rName);
    tr.onkeydown = (ev) => { if (ev.key === 'Enter' || ev.key === ' ') { ev.preventDefault(); filterMemoryByRepo(rData.display || rName); } };

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
      switchTab('telemetry');
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
      '  <div class="repo-name-wrap">' + agentTagHtml(aName) + ' <span class="text-muted" style="font-size: 0.75rem;">(' + shareStr + ')</span></div>' +
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

let recentQueryTotal = 0;

function renderRecentQueries(queries) {
  const tbody = document.getElementById('recent-tbody');
  if (!tbody) return;

  const graphQueries = queries.filter(q => GRAPH_TOOLS.includes(q.tool));
  recentQueryTotal = graphQueries.length;
  if (graphQueries.length === 0) {
    tbody.innerHTML = '<tr><td colspan="8" class="empty-cell">No graph navigation queries recorded</td></tr>';
    updateListCount('recent-list-count', 0, 0, 'newest first');
    return;
  }

  const visible = graphQueries.slice(0, FEED_LIMIT);
  tbody.innerHTML = '';
  visible.forEach(q => {
    tbody.appendChild(createRecentRow(q));
  });
  updateListCount('recent-list-count', visible.length, recentQueryTotal, 'newest first');
}

// Plain-language summary of what a command did, from its journaled tool and
// args. The raw JSON stays in the cell title for anyone who needs it.
function explainToolArgs(tool, args) {
  const a = args || {};
  const name = a.name || a.symbol || '';
  const terms = a.terms || '';
  const paths = a.paths || '';
  switch (tool) {
    case 'callers':
      return name ? 'Find who calls ' + name : 'Find who calls a function';
    case 'callees':
      return name ? 'Find what ' + name + ' calls' : 'Find what a function calls';
    case 'path':
      return a.from && a.to ? 'Route from ' + a.from + ' to ' + a.to : 'Call route between two functions';
    case 'context':
      return name ? 'Show the code around ' + name : 'Show code around a function';
    case 'impact':
      return name ? 'Show what breaks if ' + name + ' changes' : 'Show the blast radius of a change';
    case 'symbols':
      return a.file ? 'List symbols in ' + a.file : (name ? 'Find ' + name : 'Find symbols');
    case 'imports':
      return a.file ? 'Show imports of ' + a.file : 'Show the import map';
    case 'build':
      return 'Refresh the code index';
    case 'search':
      return terms ? 'Search the code for "' + terms + '"' : 'Search the code';
    case 'reuse':
      return paths ? 'Look for duplicate code in ' + paths : 'Look for duplicate code in every file';
    case 'mem_write':
      return a.key ? 'Save the note "' + a.key + '"' : 'Save a memory note';
    case 'mem_get':
      return a.key ? 'Read the note "' + a.key + '"' : 'Read a memory note';
    case 'mem_query':
      return terms ? 'Search notes for "' + terms + '"' : 'List memory notes';
    case 'mem_delete':
      return a.key ? 'Delete the note "' + a.key + '"' : 'Delete a memory note';
    case 'mem_touch':
      return a.key ? 'Mark the note "' + a.key + '" as verified' : 'Verify a memory note';
    case 'mem_export':
      return 'Back up memory notes';
    case 'mem_import':
      return 'Load memory notes from a backup';
    case 'stats':
      return 'Show repository statistics';
    case 'doctor':
      return 'Run a health check';
    case 'usage':
      return 'Show usage report';
    case 'unresolved':
      return 'List unresolved call sites';
    default:
      return '';
  }
}

function createRecentRow(q) {
  const tr = document.createElement('tr');
  tr.className = 'query-row';
  const rName = q.repo_name || UNATTRIBUTED;
  // A linked worktree keeps its own name in the feed, but scoping filters and
  // the row's provenance follow the main repository it belongs to.
  const mainName = q.repo_main_name || '';
  const aName = q.agent || 'user';
  const exitCode = parseInt(q.exit_code || 0, 10);
  tr.setAttribute('data-repo', mainName || rName);
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
    statusText = 'ambiguous';
  } else if (exitCode >= 3) {
    statusClass = 'status-err';
    statusText = 'err ' + exitCode;
  }

  const argsRaw = JSON.stringify(q.args || {});
  const argsShort = argsRaw.length > 85 ? argsRaw.substring(0, 82) + '...' : argsRaw;
  const argsHuman = explainToolArgs(q.tool, q.args) || argsShort;

  const fromMain = mainName
    ? ' <span class="text-muted wt-from">from ' + escapeHtml(mainName) + '</span>'
    : '';

  tr.innerHTML =
    '<td class="num-col text-muted">' + ts + '</td>' +
    '<td><span class="repo-tag">' + escapeHtml(rName) + '</span>' + fromMain + '</td>' +
    '<td>' + agentTagHtml(aName) + '</td>' +
    '<td><code>' + escapeHtml(q.tool || '—') + '</code></td>' +
    '<td class="text-right num-col">' + (q.shown || q.result_count || 0) + '</td>' +
    '<td class="text-right num-col">' + latStr + '</td>' +
    '<td><span class="badge ' + statusClass + '">' + statusText + '</span></td>' +
    '<td class="args-cell" title="' + escapeHtml(argsRaw) + '"><code>' + escapeHtml(argsHuman) + '</code></td>';

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
  const keep = select.value || 'ALL';
  const repos = new Set();
  reuseCandidates.forEach(c => repos.add(c.repo));

  select.innerHTML = '<option value="ALL">All Repositories</option>';
  Array.from(repos).sort().forEach(r => {
    const opt = document.createElement('option');
    opt.value = r;
    opt.textContent = r;
    select.appendChild(opt);
  });
  if (Array.from(select.options).some(o => o.value === keep)) {
    select.value = keep;
  } else {
    select.value = 'ALL';
    if (keep !== 'ALL') announce('Filter "' + keep + '" reset: repository no longer active');
  }
}

function setReuseDetectorFilter(det) {
  activeReuseDetector = det;
  document.querySelectorAll('.filter-pill[data-det]').forEach(btn => {
    btn.classList.toggle('active', btn.getAttribute('data-det') === det);
  });
  selectedReuseId = null;
  renderReuseList();
}

function getFilteredReuseCandidates() {
  const repo = document.getElementById('reuse-repo-filter') ? document.getElementById('reuse-repo-filter').value : 'ALL';
  return reuseCandidates.filter(c => {
    if (repo !== 'ALL' && c.repo !== repo) return false;
    if (activeReuseDetector !== 'ALL' && c.detector !== activeReuseDetector) return false;
    return true;
  });
}

function renderReuseList() {
  const container = document.getElementById('reuse-cards-container');
  if (!container) return;
  const filteredAll = getFilteredReuseCandidates();
  const view = viewReuseCandidates(filteredAll);
  const filtered = view.items;
  updateListCount('reuse-list-count', filtered.length, view.total, view.label);

  if (filtered.length === 0) {
    const trulyEmpty = reuseCandidates.length === 0 && activeReuseDetector === 'ALL';
    container.innerHTML = trulyEmpty
      ? '<div class="empty-cell">No duplicate candidates found in the indexed corpus.</div>'
      : '<div class="empty-cell">No candidates match the current filters.</div>';
    renderReuseDossier(null);
    return;
  }

  let selIdx = filtered.findIndex(c => c.id === selectedReuseId);
  if (selIdx < 0) {
    const stillVisible = filteredAll.some(c => c.id === selectedReuseId);
    selIdx = 0;
    selectedReuseId = filtered[0] ? filtered[0].id : null;
    if (stillVisible) announce('Selection moved: the previous candidate is outside the current view');
  }
  container.innerHTML = '';
  filtered.forEach((c, idx) => {
    const isSelected = c.id === selectedReuseId;
    const card = document.createElement('div');
    card.className = 'select-item-card' + (isSelected ? ' selected' : '');
    card.tabIndex = 0;
    card.setAttribute('role', 'button');
    card.setAttribute('aria-pressed', isSelected ? 'true' : 'false');
    card.setAttribute('aria-label', 'Candidate ' + c.symbol + ' to ' + c.canonical);
    card.onclick = () => {
      selectedReuseId = c.id;
      renderReuseList();
    };
    card.onkeydown = (ev) => {
      if (ev.key === 'Enter' || ev.key === ' ') { ev.preventDefault(); selectedReuseId = c.id; renderReuseList(); }
    };

    const detBadgeClass = 'badge badge-' + c.detector.toLowerCase();
    const repoBadge = '<span class="badge badge-repo">' + escapeHtml(c.repo) + '</span>';
    const locBadge = '<span class="badge status-warn" data-tip="card.loc_saving">-' + c.loc_saving + ' LOC</span>';
    const confBadge = c.confidence ? '<span class="badge" data-tip="card.confidence">' + escapeHtml(c.confidence) + '</span>' : '';

    card.innerHTML =
      '<div class="item-card-head">' +
      '  <div style="display:flex; align-items:center; gap:0.4rem;">' +
      '    <span class="' + detBadgeClass + '" data-tip="det.' + c.detector.toLowerCase() + '">' + c.detector + '</span>' +
          (c.overlap_applicable ? '<span class="badge" data-tip="card.overlap" style="background:var(--bg-subtle);">' + c.overlap_pct + '% overlap</span>' : '<span class="badge" data-tip="card.overlap" style="background:var(--bg-subtle);">' + escapeHtml(detectorEvidenceLabel(c.detector)) + '</span>') +
      '    ' + confBadge + locBadge +
      '  </div>' +
      '  ' + repoBadge +
      '</div>' +
      '<div class="item-key">' + escapeHtml(c.symbol) + '</div>' +
      '<div class="item-snippet">&rarr; canonical: <code>' + escapeHtml(c.canonical) + '</code> <span class="badge status-ok" data-tip="card.callers">[callers=' + c.canonical_callers + ']</span></div>' +
      '<div class="item-meta">' +
      '  <span>' + escapeHtml(c.canonical_file) + ':' + c.canonical_line + '</span>' +
      (c.shared_evidence ? '<span>&bull;</span><span>' + escapeHtml(c.shared_evidence) + '</span>' : '') +
      '<span>&bull;</span><span data-tip="card.usage">' + escapeHtml(reuseUsageText(c)) + '</span>' +
      '</div>';

    container.appendChild(card);
  });

  attachTipDescriptions(container);
  renderReuseDossier(filtered[selIdx] || null);
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
          (c.overlap_applicable ? '<span class="badge status-ok" data-tip="card.overlap">' + c.overlap_pct + '% overlap</span>' : '<span class="badge status-ok" data-tip="card.overlap">' + escapeHtml(detectorEvidenceLabel(c.detector)) + '</span>') +
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
    updateListCount('prewrite-list-count', 0, 0, 'newest first');
    return;
  }

  const visible = prewriteSearches.slice(0, FEED_LIMIT);
  tbody.innerHTML = '';
  visible.forEach(s => {
    const tr = document.createElement('tr');
    const ts = s.ts ? new Date(s.ts * 1000).toLocaleTimeString() : '—';
    const latStr = s.latency_us ? (s.latency_us / 1000.0).toFixed(1) + 'ms' : '—';
    const rName = s.repo_name || UNATTRIBUTED;
    tr.innerHTML =
      '<td class="num-col text-muted">' + ts + '</td>' +
      '<td><span class="repo-tag">' + escapeHtml(rName) + '</span></td>' +
      '<td><code>' + escapeHtml(s.tool) + '</code></td>' +
      '<td class="args-cell" title="' + escapeHtml(s.terms || '') + '"><code>' + escapeHtml(
        explainToolArgs(s.tool, s.tool === 'reuse' ? { paths: s.terms } : { terms: s.terms })
        || s.terms || 'Every file'
      ) + '</code></td>' +
      '<td class="text-right num-col">' + (s.shown || 0) + '</td>' +
      '<td class="text-right num-col">' + latStr + '</td>';
    tbody.appendChild(tr);
  });
  updateListCount('prewrite-list-count', visible.length, prewriteSearches.length, 'newest first');
}

// Agent Memory UI
function populateMemoryStoreFilter() {
  const select = document.getElementById('mem-store-filter');
  if (!select) return;
  const keep = select.value || 'ALL';
  const stores = new Map();
  allMemories.forEach(m => {
    const id = m.store_id || m.store;
    if (!stores.has(id)) stores.set(id, m.store);
  });

  select.innerHTML = '<option value="ALL">All Stores</option>';
  Array.from(stores.keys()).sort().forEach(id => {
    const opt = document.createElement('option');
    opt.value = id;
    opt.textContent = stores.get(id);
    select.appendChild(opt);
  });
  if (Array.from(select.options).some(o => o.value === keep)) {
    select.value = keep;
  } else {
    select.value = 'ALL';
    if (keep !== 'ALL') announce('Filter "' + keep + '" reset: store no longer active');
  }
}

function getFilteredMemories() {
  const store = document.getElementById('mem-store-filter') ? document.getElementById('mem-store-filter').value : 'ALL';
  const kind = document.getElementById('mem-kind-filter') ? document.getElementById('mem-kind-filter').value : 'ALL';
  const search = document.getElementById('mem-search') ? document.getElementById('mem-search').value.toLowerCase().trim() : '';
  const showSuperseded = document.getElementById('mem-show-superseded') ? document.getElementById('mem-show-superseded').checked : false;

  return allMemories.filter(m => {
    if (!showSuperseded && !m.is_current) return false;
    if (store !== 'ALL' && (m.store_id || m.store) !== store) return false;
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
  const filteredAll = getFilteredMemories();
  const view = viewMemories(filteredAll);
  const filtered = view.items;
  updateListCount('mem-list-count', filtered.length, view.total, view.label);

  if (filtered.length === 0) {
    container.innerHTML = allMemories.length === 0
      ? '<div class="empty-cell">No memory notes recorded yet. Agents write them with <code>eg mem write --body "..." --kind gotcha &lt;key&gt;</code></div>'
      : '<div class="empty-cell">No memory notes match the current filters.</div>';
    renderMemoryDossier(null);
    return;
  }

  let selIdx = filtered.findIndex(m => memorySelectionId(m) === selectedMemoryId);
  if (selIdx < 0) {
    const stillVisible = filteredAll.some(m => memorySelectionId(m) === selectedMemoryId);
    selIdx = 0;
    selectedMemoryId = filtered[0] ? memorySelectionId(filtered[0]) : null;
    if (stillVisible) announce('Selection moved: the previous note is outside the current view');
  }
  container.innerHTML = '';
  filtered.forEach((m) => {
    const isSelected = memorySelectionId(m) === selectedMemoryId;
    const card = document.createElement('div');
    card.className = 'select-item-card' + (isSelected ? ' selected' : '');
    card.tabIndex = 0;
    card.setAttribute('role', 'button');
    card.setAttribute('aria-pressed', isSelected ? 'true' : 'false');
    card.setAttribute('aria-label', 'Memory note ' + m.key);
    card.onclick = () => {
      selectedMemoryId = memorySelectionId(m);
      renderMemoryList();
    };
    card.onkeydown = (ev) => {
      if (ev.key === 'Enter' || ev.key === ' ') { ev.preventDefault(); selectedMemoryId = memorySelectionId(m); renderMemoryList(); }
    };

    const kindClass = 'badge status-' + (m.kind === 'decision' ? 'ambig' : m.kind === 'gotcha' ? 'warn' : m.kind === 'fact' ? 'ok' : m.kind === 'procedure' ? 'neutral' : 'err');
    const createdStr = m.created_at ? new Date(m.created_at * 1000).toLocaleDateString() : '—';
    const lastUsedStr = m.last_recalled_ts
      ? new Date(m.last_recalled_ts * 1000).toLocaleString([], { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' })
      : 'never';
    const verifiedStr = m.anchored_at
      ? new Date(m.anchored_at * 1000).toLocaleDateString()
      : '';
    const storeBadge = '<span class="badge badge-repo">' + escapeHtml(m.store) + '</span>';
    const revPill = '<span class="badge" style="background: var(--bg-subtle);">r' + m.rev + '</span>';
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
      '    ' + refBadge +
      '  </div>' +
      '  ' + storeBadge +
      '</div>' +
      '<div class="item-key">' + escapeHtml(m.key) + '</div>' +
      '<div class="item-snippet">' + escapeHtml(m.body) + '</div>' +
      '<div class="item-meta">' +
      '  <span>' + escapeHtml(m.scope) + '</span>' +
      '  <span>&bull;</span>' +
      '  <span>created ' + createdStr + '</span>' +
      '  <span>&bull;</span>' +
      '  <span data-tip="card.recalled">recalled ' + (m.recall_count || 0) + '&times;</span>' +
      '  <span>&bull;</span>' +
      '  <span>last used ' + lastUsedStr + '</span>' +
      (verifiedStr ? '<span>&bull;</span><span data-tip="card.verified">verified ' + verifiedStr + '</span>' : '') +
      (m.refs && m.refs.length ? '<span>&bull;</span><span>' + m.refs.length + (m.refs.length === 1 ? ' ref' : ' refs') + '</span>' : '') +
      '</div>';

    container.appendChild(card);
  });
  attachTipDescriptions(container);

  renderMemoryDossier(filtered[selIdx] || null);
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
      const status = r.status || (r.verified ? 'valid' : 'broken');
      const verClass = status === 'broken' ? 'chip-ref broken'
        : status === 'modified' ? 'chip-ref modified'
        : status === 'unverified' ? 'chip-ref unverified'
        : 'chip-ref verified';
      const verBadge = status === 'broken' ? ' [BROKEN REF]'
        : status === 'modified' ? ' [MODIFIED REF]'
        : status === 'unverified' ? ' [UNVERIFIED — NO INDEX]'
        : ' [VERIFIED ' + escapeHtml(r.type || 'file').toUpperCase() + ']';
      const locStr = r.location ? ' &middot; ' + escapeHtml(r.location) : '';
      refsHtml += '<span class="' + verClass + '"><strong>' + escapeHtml(r.ref) + '</strong>' + verBadge + locStr + '</span>';
    });
    refsHtml += '</div>';
  } else if (m.refs && m.refs.length) {
    refsHtml = '<div class="dossier-section-title">Referenced Symbols &amp; Paths</div><div class="chips-row">';
    m.refs.forEach(r => { refsHtml += '<span class="chip chip-ref">' + escapeHtml(r) + '</span>'; });
    refsHtml += '</div>';
  }

  const touchBtn = m.ref_status === 'modified' && m.is_current
    ? '<button class="btn-touch" style="padding: 0.15rem 0.5rem; font-size: 0.72rem; margin-left: 0.5rem;" data-key="' + escapeHtml(m.key) + '" data-store-id="' + escapeHtml(m.store_id || m.store) + '" data-rev="' + m.rev + '">Verify</button>'
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

let isDocumentVisible = true;

// Selective Invalidation Debounce Timers
let summaryDebounceTimer = null;
let reuseDebounceTimer = null;
let memoriesDebounceTimer = null;

// Screen-reader announcements: state transitions only, never ordinary rows.
let lastAnnounce = 0;
function announce(text) {
  const region = document.getElementById('live-region');
  if (!region) return;
  const now = Date.now();
  if (now - lastAnnounce < 1000) return;
  lastAnnounce = now;
  region.textContent = text;
}

function showFetchError(tab, message) {
  const el = document.getElementById('data-error-' + tab);
  if (!el) return;
  el.textContent = 'Data unavailable: ' + message + ' — retry with the button.';
  el.classList.remove('hidden');
  if (!el.querySelector('button')) {
    const btn = document.createElement('button');
    btn.className = 'btn-touch';
    btn.style.marginLeft = '0.5rem';
    btn.textContent = 'Retry';
    btn.addEventListener('click', () => {
      if (tab === 'telemetry') loadSummaryData();
      else if (tab === 'reuse') loadReuseData();
      else loadMemoriesData();
    });
    el.appendChild(btn);
  }
}

function clearFetchError(tab) {
  const el = document.getElementById('data-error-' + tab);
  if (el) {
    el.classList.add('hidden');
    el.textContent = '';
  }
}

function setDataAge(generatedAt) {
  const el = document.getElementById('data-age');
  if (!el) return;
  if (!generatedAt) {
    el.textContent = '';
    return;
  }
  el.textContent = 'Data as of ' + new Date(generatedAt * 1000).toLocaleTimeString();
}

function triggerSummaryRefresh() {
  // No reset on later events: the timer fires once per window. A trailing
  // debounce that resets on every event starves under a steady stream.
  if (summaryDebounceTimer) return;
  summaryDebounceTimer = setTimeout(() => {
    summaryDebounceTimer = null;
    loadSummaryData();
  }, 3000);
}

function triggerReuseRefresh() {
  if (activeTab !== 'reuse') return;
  if (reuseDebounceTimer) return;
  reuseDebounceTimer = setTimeout(() => {
    reuseDebounceTimer = null;
    loadReuseData();
  }, 1000);
}

function triggerMemoriesRefresh() {
  if (memoriesDebounceTimer) return;
  memoriesDebounceTimer = setTimeout(() => {
    memoriesDebounceTimer = null;
    loadMemoriesData();
  }, 500);
}

function flashElement(el) {
  if (!el) return;
  el.classList.remove('metric-flash');
  void el.offsetWidth;
  el.classList.add('metric-flash');
}

function handleLiveEvent(q) {
  const tool = q.tool || '';
  const isGraph = GRAPH_TOOLS.includes(tool);
  const exitCode = parseInt(q.exit_code || 0, 10);

  // Numbers are owned by the server payload: the scheduler refetches, and the
  // rows below are the only client-side state.
  if (isGraph) {
    const recentTbody = document.getElementById('recent-tbody');
    if (recentTbody) {
      const emptyRow = recentTbody.querySelector('.empty-cell');
      if (emptyRow) recentTbody.innerHTML = '';
      const row = createRecentRow(q);
      recentTbody.insertBefore(row, recentTbody.firstChild);
      recentQueryTotal += 1;
      while (recentTbody.children.length > FEED_LIMIT) {
        recentTbody.removeChild(recentTbody.lastChild);
      }
      updateListCount('recent-list-count', recentTbody.children.length, recentQueryTotal, 'newest first');
      applyTelemetryFilters();
    }
    flashElement(document.getElementById('metric-total-calls'));
  } else if (tool === 'search' || tool === 'reuse') {
    const args = q.args || {};
    const terms = args.terms || args.paths || '';
    prewriteSearches.unshift({
      ts: q.ts,
      repo_name: q.repo_name,
      tool: tool,
      terms: terms,
      shown: q.shown || q.result_count || 0,
      latency_us: q.latency_us || 0,
    });
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
    flashElement(document.getElementById('mem-kpi-currency'));
  }

  // Bounded reconciliation: debounce without reset, so a steady event stream
  // still refreshes at least every window instead of starving the timer.
  triggerSummaryRefresh();
  if (tool === 'build') {
    triggerReuseRefresh();
  } else if (tool === 'mem_write' || tool === 'mem_delete' || tool === 'mem_touch') {
    triggerMemoriesRefresh();
  }
}

function escapeHtml(str) {
  return String(str || '')
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');
}

// Hover help. One copy table, one shared node, delegated listeners.
const TIP = {
  'kpi.graph_queries': 'Code lookups agents ran against the index instead of reading files.',
  'kpi.p50': 'Half of all queries finished faster than this.',
  'kpi.tokens_saved': 'Estimated tokens an agent avoided spending on source reads (coefficients, not a measurement).',
  'kpi.candidates': 'Functions that look like near-copies of another function.',
  'kpi.excess_loc': 'Estimated deletable lines, extrapolated from the sampled candidates.',
  'kpi.recall': 'Share of tagged sessions (ENGRAM_SESSION set) whose calls read memory. Untagged sessions are not measured.',
  'kpi.currency': 'Share of current notes whose code refs resolve to the current index. Notes without refs are excluded; notes in stores without an index are unverified.',
  'col.last_active': 'Most recent run recorded for this row.',
  'col.p50': 'Half of runs were faster than this.',
  'col.items': 'Results returned by the query.',
  'col.reliability': 'Share of runs that finished without error.',
  'col.tokens_saved': 'Estimated tokens saved by search and reuse runs together.',
  'col.hits': 'Search results shown to the agent in that repository.',
  'col.reads_writes': 'Memory reads versus notes written.',
  'card.overlap': 'Measured evidence overlap for D1/D2; D3/D4 show their evidence class instead of a percentage.',
  'card.callers': 'Call sites using the canonical helper.',
  'card.confidence': 'Static confidence tier; stale or unknown freshness caps it at low.',
  'card.usage': 'How many eg reuse runs in the retained journal surfaced this candidate, and when the last one ran.',
  'card.recalled': 'Exact-key mem_get recalls in the retained journal.',
  'card.verified': 'Last Verify (anchor) timestamp; the creation date never changes.',
  'view.memory': 'Collapse the note list: Top 10 or 3 by latest activity (created, verified, or recalled), most recalled, or all notes.',
  'view.reuse': 'Collapse the candidate list by canonical call-site usage, or show all sampled candidates.',
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
  loadMemoriesData();
  initEventSource();
  initDelegation();
  initTabKeyboard();
});

// One delegated listener replaces every inline handler (CSP: no unsafe-inline).
function initDelegation() {
  document.addEventListener('click', (e) => {
    const themeBtn = e.target.closest('#theme-toggle-btn');
    if (themeBtn) { toggleTheme(); return; }
    const tabBtn = e.target.closest('[data-tab]');
    if (tabBtn) { switchTab(tabBtn.dataset.tab); return; }
    const pill = e.target.closest('[data-det]');
    if (pill) { setReuseDetectorFilter(pill.dataset.det); return; }
    const touchBtn = e.target.closest('.btn-touch[data-key]');
    if (touchBtn) { touchMemory(touchBtn); return; }
    const verifyAll = e.target.closest('#verify-all-btn');
    if (verifyAll) { verifyAllModified(); return; }
  });
  document.addEventListener('input', (e) => {
    const id = e.target.id;
    if (id === 'query-search') applyTelemetryFilters();
    else if (id === 'mem-search') renderMemoryList();
  });
  document.addEventListener('change', (e) => {
    const id = e.target.id;
    if (id === 'repo-filter' || id === 'agent-filter' || id === 'status-filter') applyTelemetryFilters();
    else if (id === 'reuse-repo-filter') renderReuseList();
    else if (id === 'mem-store-filter' || id === 'mem-kind-filter' || id === 'mem-show-superseded') renderMemoryList();
    else if (id === 'mem-view' && MEMORY_VIEWS[e.target.value]) { memView = e.target.value; renderMemoryList(); }
    else if (id === 'reuse-view' && REUSE_VIEWS[e.target.value]) { reuseView = e.target.value; renderReuseList(); }
  });
}

function initTabKeyboard() {
  const nav = document.querySelector('.nav-tabs');
  if (!nav) return;
  nav.addEventListener('keydown', (e) => {
    if (!['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(e.key)) return;
    const idx = TAB_KEYS.indexOf(activeTab);
    let next = idx;
    if (e.key === 'ArrowLeft') next = (idx + TAB_KEYS.length - 1) % TAB_KEYS.length;
    else if (e.key === 'ArrowRight') next = (idx + 1) % TAB_KEYS.length;
    else if (e.key === 'Home') next = 0;
    else if (e.key === 'End') next = TAB_KEYS.length - 1;
    e.preventDefault();
    switchTab(TAB_KEYS[next]);
    const btn = document.getElementById('tab-' + activeTab + '-btn');
    if (btn) btn.focus();
  });
}

function refreshVisibleData() {
  loadSummaryData();
  if (activeTab === 'reuse') loadReuseData();
  if (activeTab === 'memory') loadMemoriesData();
}

function initEventSource() {
  const liveBadge = document.getElementById('live-indicator');
  if (liveBadge) liveBadge.style.display = 'inline-block';
  let isDocumentVisible = !document.hidden;

  const source = new EventSource('/events');

  source.onopen = function () {
    const dot = document.querySelector('.live-dot');
    if (dot) dot.className = 'live-dot live-active';
    if (liveBadge) {
      liveBadge.title = 'Live stream connected';
      liveBadge.className = 'badge status-ok';
      liveBadge.lastChild.nodeValue = 'LIVE';
    }
    // Events missed while disconnected are covered by a full refetch.
    refreshVisibleData();
  };

  source.onerror = function () {
    const dot = document.querySelector('.live-dot');
    if (dot) dot.className = 'live-dot live-reconnecting';
    if (liveBadge) {
      liveBadge.title = 'Reconnecting to live stream...';
      liveBadge.className = 'badge status-warn';
      liveBadge.lastChild.nodeValue = 'RECONNECTING';
    }
  };

  source.addEventListener('reset', function () {
    announce('Live stream reset; data reloaded.');
    refreshVisibleData();
  });

  source.onmessage = function (e) {
    try {
      const q = JSON.parse(e.data);
      handleLiveEvent(q);
    } catch (err) {}
  };

  document.addEventListener('visibilitychange', () => {
    isDocumentVisible = !document.hidden;
    const dot = document.querySelector('.live-dot');
    if (isDocumentVisible) {
      if (dot) dot.className = 'live-dot live-active';
      refreshVisibleData();
    } else if (dot) {
      dot.className = 'live-dot live-paused';
    }
  });

  // Fallback poll: keeps KPIs fresh when the stream is quiet or degraded.
  setInterval(() => {
    if (!isDocumentVisible) return;
    refreshVisibleData();
  }, 15000);
}
