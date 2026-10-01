import { GraphScene } from './scene/GraphScene.ts';
import { decodeBinaryGraph } from './wire/decoder.ts';
import { HeaderControls, SearchMode } from './ui/HeaderControls.ts';
import { FilterPanel } from './ui/FilterPanel.ts';
import { Inspector } from './ui/Inspector.ts';
import { TelemetryFeed } from './ui/TelemetryFeed.ts';
import { AgentActivation, CorpusMetadata, GraphPayload, NodeData, ViewMode } from './types.ts';

class GraphViewApp {
  private scene: GraphScene;
  private header: HeaderControls;
  private filterPanel: FilterPanel;
  private inspector: Inspector;
  private telemetry: TelemetryFeed;

  private corpora: CorpusMetadata[] = [];
  private activeCorpus = 'all';
  private previousCorpus = 'all';
  private isEgoFocused = false;
  private currentViewMode: ViewMode = 'entity';
  private currentSearchMode: SearchMode = 'symbol';
  private currentPayload: GraphPayload | null = null;
  private currentSearchMatches: Set<number> | null = null;
  private searchDebounceTimer: number | null = null;

  constructor() {
    const canvasContainer = document.getElementById('canvas-container')!;
    this.scene = new GraphScene(canvasContainer);

    this.header = new HeaderControls(document.getElementById('header-mount')!);
    this.filterPanel = new FilterPanel(document.getElementById('filter-mount')!);
    this.inspector = new Inspector(document.getElementById('inspector-mount')!);
    this.telemetry = new TelemetryFeed(document.getElementById('telemetry-mount')!);

    (window as any).__graphApp = this;

    this.bindEvents();
  }

  private bindEvents() {
    // 1. Header Events
    this.header.onViewModeChange = (mode) => {
      this.currentViewMode = mode;
      this.scene.nodeCloud.setViewMode(mode);
    };

    this.header.onCorpusChange = (corpus) => {
      this.activeCorpus = corpus;
      this.currentSearchMatches = null;
      this.loadGraph();
    };

    this.header.onReload = async () => {
      const target = this.activeCorpus === 'all' ? 'overview' : this.activeCorpus;
      try {
        await fetch(`/api/graph/reload/${target}`, { method: 'POST' });
        console.log(`[GraphView] Invalidated cache for ${target}`);
        await this.loadGraph();
      } catch (err) {
        console.error('Failed to reload corpus:', err);
      }
    };

    this.header.onUnfocus = () => {
      this.unfocusGraph();
    };

    this.header.onQuery = (query) => {
      this.performSearch(query);
    };

    this.header.onQuerySubmit = (query) => {
      this.performSearchSubmit(query);
    };

    this.header.onToggleLeftSidebar = (open) => {
      const mount = document.getElementById('filter-mount');
      mount?.classList.toggle('collapsed', !open);
      document.body.classList.toggle('left-sidebar-collapsed', !open);
    };

    this.header.onToggleRightSidebar = (open) => {
      this.telemetry.setCollapsed(!open);
      document.body.classList.toggle('right-sidebar-collapsed', !open);
    };

    this.header.onZoomIn = () => {
      this.scene.camera.position.multiplyScalar(0.8);
      this.scene.controls.update();
    };

    this.header.onZoomOut = () => {
      this.scene.camera.position.multiplyScalar(1.25);
      this.scene.controls.update();
    };

    this.header.onFitView = () => {
      this.scene.camera.position.set(0, 450, 1200);
      this.scene.controls.target.set(0, 0, 0);
      this.scene.controls.update();
    };

    this.header.onSearchModeChange = (mode) => {
      this.currentSearchMode = mode;
      const queryInput = document.getElementById('query-input') as HTMLInputElement;
      if (queryInput && queryInput.value) {
        this.performSearch(queryInput.value);
      }
    };

    // 2. FilterPanel Events
    this.filterPanel.onEntityFilter = (category) => {
      this.scene.setEntityFilter(category);
    };

    this.filterPanel.onEdgeClassToggle = (cls, enabled) => {
      this.scene.edgeLines.toggleClass(cls, enabled);
    };

    this.filterPanel.onEdgeTypeToggle = (type, enabled) => {
      this.scene.edgeLines.toggleType(type, enabled);
    };

    this.filterPanel.onParticleSizeChange = (val) => {
      this.scene.setParticleScale(val);
    };

    this.filterPanel.onBloomChange = (val) => {
      this.scene.setBloomStrength(val);
    };

    this.filterPanel.onBloomThresholdChange = (val) => {
      this.scene.setBloomThreshold(val);
    };

    this.filterPanel.onExposureChange = (val) => {
      this.scene.setExposure(val);
    };

    this.filterPanel.onGradientContrastToggle = (enabled) => {
      this.scene.setGradientContrast(enabled);
    };

    this.filterPanel.onEdgeDensityChange = (val) => {
      this.scene.setEdgeDensity(val);
    };

    this.filterPanel.onAutoOrbitToggle = (enabled) => {
      this.scene.setAutoRotate(enabled);
    };

    this.filterPanel.onOrbitSpeedChange = (speed) => {
      this.scene.setAutoRotate(true, speed);
    };



    this.filterPanel.onLabelSizeChange = (val) => {
      this.scene.corpusLabels.setSize(val);
    };

    this.filterPanel.onLabelBrightnessChange = (val) => {
      this.scene.corpusLabels.setBrightness(val);
    };

    this.filterPanel.onActivationSizeChange = (val) => {
      this.scene.setActivationScale(val);
    };

    this.filterPanel.onCommunityGravityChange = (val) => {
      this.scene.updateGravity({ communityGravity: val });
    };

    this.filterPanel.onCorpusGravityChange = (val) => {
      this.scene.updateGravity({ corpusGravity: val });
    };

    this.filterPanel.onInterCorpusAttractionChange = (val) => {
      this.scene.updateGravity({ interCorpusAttraction: val });
    };

    // 3. Scene Interaction
    this.scene.onNodeClick = (node: NodeData) => {
      this.inspector.showNode(node, this.isEgoFocused);
      this.scene.flyTo(node.position, 200);
    };

    this.scene.onBackgroundClick = () => {
      if (this.isEgoFocused) {
        this.unfocusGraph();
      } else {
        this.scene.nodeCloud.setSelectedId(null);
        this.inspector.showNode(null);
      }
    };

    this.inspector.onFocusEgo = (node: NodeData) => {
      this.loadEgoSubgraph(node.path);
    };

    this.inspector.onUnfocus = () => {
      this.unfocusGraph();
    };

    this.inspector.onClose = () => {
      if (this.isEgoFocused) {
        this.unfocusGraph();
      }
    };

    this.inspector.onReadSource = (node: NodeData) => {
      this.readNodeSource(node.path);
    };

    // 4. Telemetry Stream
    this.telemetry.onActivation = (act) => {
      if (!this.currentPayload) return;
      let matched = this.findMatchingNodes(act.paths || []);

      // If paths yielded no direct matches, attempt keyword matching on query terms
      if (matched.length === 0 && act.query) {
        const queryTerms = act.query
          .replace(/[:"'{}\[\]()=]/g, ' ')
          .split(/\s+/)
          .filter(
            (t) =>
              t.length >= 3 &&
              !['search', 'match', 'graph', 'corpus', 'mode', 'calls', 'defines', 'imports', 'implements'].includes(
                t.toLowerCase()
              )
          );
        if (queryTerms.length > 0) {
          matched = this.findMatchingNodes(queryTerms);
        }
      }

      let fallbackPos: [number, number, number] | undefined = undefined;
      if (matched.length === 0 && act.corpus && act.corpus !== 'all') {
        const corpMeta = this.corpora.find((c) => c.name === act.corpus);
        if (corpMeta && corpMeta.center) {
          fallbackPos = corpMeta.center;
        }
      }

      if (matched.length > 0 || fallbackPos) {
        this.scene.triggerActivationEffect(act, matched, fallbackPos);
        if (matched.length > 0) {
          this.scene.nodeCloud.setHoveredId(matched[0].id);
        }
      }
    };

    this.telemetry.onCollapseToggle = (collapsed) => {
      this.header.setRightSidebarState(!collapsed);
      document.body.classList.toggle('right-sidebar-collapsed', collapsed);
    };

    this.telemetry.onSelectActivation = (act) => {
      this.handleActivationSelect(act);
    };

    this.telemetry.onSelectPaths = (paths) => {
      if (!this.currentPayload || paths.length === 0) return;
      const matched = this.findMatchingNodes(paths);
      if (matched.length > 0) {
        const primary = matched[0];
        this.scene.nodeCloud.setSelectedId(primary.id);
        this.scene.flyTo(primary.position, 220);
        this.inspector.showNode(primary);
      }
    };
  }

  private async performSearch(rawQuery: string, triggerTelemetry: boolean = true) {
    if (this.searchDebounceTimer) {
      clearTimeout(this.searchDebounceTimer);
      this.searchDebounceTimer = null;
    }

    const q = (rawQuery || '').trim();
    if (!q || !this.currentPayload) {
      this.currentSearchMatches = null;
      this.scene.setSearchMatches(null);
      this.header.setQueryStatus('idle');
      return;
    }

    const startTime = performance.now();
    const qLower = q.toLowerCase();

    // Auto-detect mode if query has syntax prefix
    let mode = this.currentSearchMode;
    if (
      qLower.startsWith('calls:') ||
      qLower.startsWith('defines:') ||
      qLower.startsWith('imports:') ||
      qLower.startsWith('implements:') ||
      qLower.startsWith('edge:')
    ) {
      mode = 'graph';
      this.header.setSearchMode('graph');
    } else if (qLower.startsWith('read:') || qLower.startsWith('/read')) {
      mode = 'read';
      this.header.setSearchMode('read');
    }

    const matches = new Set<number>();

    if (mode === 'graph') {
      // AST graph relation search across Petgraph edges
      const edgeFilter = qLower.startsWith('calls:')
        ? 'calls'
        : qLower.startsWith('defines:')
        ? 'defines'
        : qLower.startsWith('imports:')
        ? 'imports'
        : qLower.startsWith('implements:')
        ? 'implements'
        : qLower.startsWith('edge:')
        ? qLower.replace('edge:', '').trim()
        : qLower;
      const targetSub = qLower.includes(':') ? qLower.split(':')[1].trim() : '';

      for (const edge of this.currentPayload.edges) {
        if (edge.edgeType.toLowerCase().includes(edgeFilter)) {
          const srcNode = this.currentPayload.nodes.find((n) => n.id === edge.source);
          const tgtNode = this.currentPayload.nodes.find((n) => n.id === edge.target);
          if (srcNode && tgtNode) {
            if (
              !targetSub ||
              srcNode.path.toLowerCase().includes(targetSub) ||
              tgtNode.path.toLowerCase().includes(targetSub)
            ) {
              matches.add(srcNode.id);
              matches.add(tgtNode.id);
            }
          }
        }
      }
    } else {
      // Standard symbol / substring search
      for (const node of this.currentPayload.nodes) {
        const matchPath = node.path && node.path.toLowerCase().includes(qLower);
        const matchTitle = node.title && node.title.toLowerCase().includes(qLower);
        const matchType = node.entityType && node.entityType.toLowerCase().includes(qLower);
        if (matchPath || matchTitle || matchType) {
          matches.add(node.id);
        }
      }
    }

    this.currentSearchMatches = matches;
    this.scene.setSearchMatches(matches);

    if (matches.size > 0) {
      this.header.setQueryStatus('matched');
    } else {
      this.header.setQueryStatus('nomatch');
    }

    if (triggerTelemetry) {
      this.searchDebounceTimer = window.setTimeout(() => {
        this.searchDebounceTimer = null;
        if (!this.currentPayload) return;

        const matchedNodes = this.currentPayload.nodes.filter((n) =>
          matches.has(n.id)
        );
        const matchedPaths = matchedNodes.map((n) => n.path);
        const elapsed = Math.round(performance.now() - startTime);

        this.telemetry.recordActivation({
          timestamp: Date.now(),
          tool: mode === 'graph' ? 'graph_match' : 'search',
          client_id: 'user',
          client_name: 'Search',
          client_color: mode === 'graph' ? '#10b981' : '#38bdf8',
          corpus: this.activeCorpus,
          query: q,
          paths: matchedPaths.slice(0, 15),
          duration_ms: elapsed,
          success: matches.size > 0,
        });
      }, 450);
    }
  }

  private async performSearchSubmit(rawQuery: string) {
    if (this.searchDebounceTimer) {
      clearTimeout(this.searchDebounceTimer);
      this.searchDebounceTimer = null;
    }

    const q = (rawQuery || '').trim();
    if (!q || !this.currentPayload) return;

    const startTime = performance.now();
    await this.performSearch(q, false);

    const qLower = q.toLowerCase();
    const isRead =
      this.currentSearchMode === 'read' ||
      qLower.startsWith('read:') ||
      qLower.startsWith('/read');

    if (isRead) {
      const cleanPath = q.replace(/^read:|^[\/]read\s*/i, '').trim();
      await this.readNodeSource(cleanPath);
      return;
    }

    if (!this.currentSearchMatches || this.currentSearchMatches.size === 0) return;

    // Collect matched nodes
    const matchedNodes = this.currentPayload.nodes.filter((n) =>
      this.currentSearchMatches!.has(n.id)
    );
    const matchedPaths = matchedNodes.map((n) => n.path);
    const elapsed = Math.round(performance.now() - startTime);

    // Record synthetic activation into Agent Activity Feed
    this.telemetry.recordActivation({
      timestamp: Date.now(),
      tool: this.currentSearchMode === 'graph' ? 'graph_match' : 'search',
      client_id: 'user',
      client_name: 'Search',
      client_color: this.currentSearchMode === 'graph' ? '#10b981' : '#38bdf8',
      corpus: this.activeCorpus,
      query: q,
      paths: matchedPaths.slice(0, 15),
      duration_ms: elapsed,
      success: true,
    });

    // Fly to first match and inspect
    if (matchedNodes.length > 0) {
      const first = matchedNodes[0];
      this.scene.nodeCloud.setSelectedId(first.id);
      this.scene.flyTo(first.position, 220);
      this.inspector.showNode(first);
    }
  }

  private async handleActivationSelect(act: AgentActivation) {
    if (!this.currentPayload) return;

    // Update search input if query was recorded without retriggering search query event
    if (act.query) {
      this.header.setSearchQuery(act.query, false);
    }

    // Match nodes by path using robust matching
    const matchedNodes = this.findMatchingNodes(act.paths);

    // Trigger rich 3D animation (pulse / explosion / traversal / scanner)
    this.scene.triggerActivationEffect(act, matchedNodes);

    if (matchedNodes.length > 0) {
      const matchIds = new Set(matchedNodes.map((n) => n.id));
      this.currentSearchMatches = matchIds;
      this.scene.setSearchMatches(matchIds);
      this.header.setQueryStatus('matched');

      // Compute centroid of matched nodes
      let cx = 0, cy = 0, cz = 0;
      for (const n of matchedNodes) {
        cx += n.position[0];
        cy += n.position[1];
        cz += n.position[2];
      }
      cx /= matchedNodes.length;
      cy /= matchedNodes.length;
      cz /= matchedNodes.length;

      this.scene.flyTo([cx, cy, cz], matchedNodes.length > 1 ? 380 : 220);
      this.scene.nodeCloud.setSelectedId(matchedNodes[0].id);
      this.inspector.showNode(matchedNodes[0]);
    }
  }

  private async readNodeSource(path: string) {
    try {
      this.showLoading(`Reading ${path}...`);
      const corpusScope =
        this.activeCorpus === 'all' || this.activeCorpus === 'overview' || this.activeCorpus === 'ego'
          ? ''
          : this.activeCorpus;
      const url = `/api/graph/read?path=${encodeURIComponent(path)}&corpus=${encodeURIComponent(corpusScope)}`;
      const res = await fetch(url);
      this.hideLoading();
      if (!res.ok) {
        console.error(`Failed to read node ${path}: HTTP ${res.status}`);
        return;
      }
      const data = await res.json();
      this.inspector.showSourceCode(data.content, data.file_path, data.start_line, data.language);
    } catch (err) {
      this.hideLoading();
      console.error('Error reading node source:', err);
    }
  }

  public async init() {
    try {
      await this.fetchStatus();
      this.header.render(
        this.corpora,
        this.activeCorpus,
        this.currentViewMode,
        0,
        0
      );
      await this.loadGraph();
      this.telemetry.start();
    } catch (err) {
      console.error('Failed to initialize GraphView:', err);
      this.showLoadingError(String(err));
    }
  }

  private async fetchStatus() {
    const res = await fetch('/api/status');
    if (!res.ok) return;
    const data = await res.json();
    this.corpora = data.corpora || [];
  }

  private showLoading(text: string) {
    const overlay = document.getElementById('loading-overlay');
    const label = document.getElementById('loading-text');
    if (overlay) overlay.classList.remove('hidden');
    if (label) label.textContent = text;
  }

  private hideLoading() {
    const overlay = document.getElementById('loading-overlay');
    if (overlay) overlay.classList.add('hidden');
  }

  private showLoadingError(err: string) {
    const overlay = document.getElementById('loading-overlay');
    const label = document.getElementById('loading-text');
    if (overlay) overlay.classList.remove('hidden');
    if (label) {
      label.innerHTML = `<span style="color:#ef4444">Error loading graph:</span><br/><span style="color:#94a3b8;font-size:11px">${err}</span>`;
    }
  }

  private async unfocusGraph() {
    this.isEgoFocused = false;
    this.activeCorpus = this.previousCorpus === 'ego' ? 'all' : this.previousCorpus;
    this.inspector.showNode(null);
    this.scene.nodeCloud.setSelectedId(null);
    await this.loadGraph();
  }

  private async loadGraph() {
    if (this.searchDebounceTimer) {
      clearTimeout(this.searchDebounceTimer);
      this.searchDebounceTimer = null;
    }
    if (this.activeCorpus !== 'ego') {
      this.isEgoFocused = false;
    }
    const targetLabel = this.activeCorpus === 'all' ? 'All Corpora' : this.activeCorpus;
    this.showLoading(`Loading ${targetLabel}...`);

    try {
      const url =
        this.activeCorpus === 'all'
          ? `/api/graph/overview?cluster_mode=community`
          : `/api/graph/corpus/${encodeURIComponent(this.activeCorpus)}?cluster_mode=community`;

      const res = await fetch(url);
      if (!res.ok) throw new Error(`HTTP ${res.status} loading graph`);

      const buffer = await res.arrayBuffer();
      const payload = decodeBinaryGraph(buffer);
      payload.corpus = this.activeCorpus;

      if (this.activeCorpus === 'all' || this.activeCorpus === 'overview') {
        for (const node of payload.nodes) {
          const cIdx = Math.floor(node.community / 1000) - 1;
          if (cIdx >= 0 && cIdx < this.corpora.length) {
            node.corpus = this.corpora[cIdx].name;
          } else {
            node.corpus = 'default';
          }
        }
      } else {
        for (const node of payload.nodes) {
          node.corpus = this.activeCorpus;
        }
      }

      this.currentPayload = payload;

      this.scene.setData(payload, this.corpora, this.currentViewMode);
      this.header.render(
        this.corpora,
        this.activeCorpus,
        this.currentViewMode,
        payload.nodes.length,
        payload.edges.length
      );
      this.filterPanel.render(payload.nodes, payload.edges);
      this.hideLoading();

      // If active search exists, re-apply
      if (this.currentSearchMatches) {
        this.scene.setSearchMatches(this.currentSearchMatches);
      }
    } catch (err) {
      this.showLoadingError(String(err));
      throw err;
    }
  }

  private async loadEgoSubgraph(centerPath: string) {
    if (this.activeCorpus !== 'ego') {
      this.previousCorpus = this.activeCorpus;
    }
    this.isEgoFocused = true;
    this.showLoading(`Extracting ego subgraph for ${centerPath}...`);
    try {
      const url = `/api/graph/subgraph?center=${encodeURIComponent(centerPath)}&hops=2&budget=120&cluster_mode=community`;
      const res = await fetch(url);
      if (!res.ok) {
        console.error(`Failed to load ego subgraph for ${centerPath}: HTTP ${res.status}`);
        this.hideLoading();
        return;
      }

      const buffer = await res.arrayBuffer();
      const payload = decodeBinaryGraph(buffer);
      payload.corpus = 'ego';
      this.currentPayload = payload;

      this.scene.setData(payload, this.corpora, this.currentViewMode);
      this.header.render(this.corpora, 'ego', this.currentViewMode, payload.nodes.length, payload.edges.length);
      this.filterPanel.render(payload.nodes, payload.edges);
      this.hideLoading();

      // Find center node in ego subgraph, select it and show in inspector with isEgoFocused = true
      const normCenter = centerPath.replace(/\\/g, '/');
      const centerNode = payload.nodes.find((n) => n.path.replace(/\\/g, '/') === normCenter);
      if (centerNode) {
        this.scene.nodeCloud.setSelectedId(centerNode.id);
        this.inspector.showNode(centerNode, true);
        this.scene.flyTo(centerNode.position, 220);
      }
    } catch (err) {
      console.error('Error loading ego subgraph:', err);
      this.hideLoading();
    }
  }

  private findMatchingNodes(paths: string[]): NodeData[] {
    if (!this.currentPayload || !paths || paths.length === 0) return [];
    const normalizedTargets: string[] = [];
    for (const p of paths) {
      if (!p) continue;
      let clean = p.replace(/\\/g, '/').toLowerCase().trim();
      clean = clean.replace(/[:#]l?\d+.*$/i, '').trim();
      if (clean) normalizedTargets.push(clean);
      if (clean.includes('#')) {
        const parts = clean.split('#');
        if (parts[0]) normalizedTargets.push(parts[0]);
        if (parts[1]) normalizedTargets.push(parts[1]);
      }
    }
    const results: NodeData[] = [];
    const seen = new Set<number>();

    for (const target of normalizedTargets) {
      const targetBase = target.split('/').pop() || target;
      const targetSym = target.split('#').pop() || targetBase;
      for (const node of this.currentPayload.nodes) {
        if (seen.has(node.id)) continue;
        const nPath = (node.path || '').replace(/\\/g, '/').toLowerCase();
        const nTitle = (node.title || '').toLowerCase();
        const nBase = nPath.split('/').pop() || nPath;
        const nSym = nPath.split('#').pop() || nBase;

        if (
          nPath === target ||
          nPath.endsWith(target) ||
          target.endsWith(nPath) ||
          nTitle === target ||
          nTitle === targetBase ||
          nTitle === targetSym ||
          nSym === targetSym ||
          nBase === targetBase
        ) {
          seen.add(node.id);
          results.push(node);
        }
      }
    }

    if (results.length === 0) {
      for (const target of normalizedTargets) {
        const targetBase = target.split('/').pop() || target;
        const targetSym = target.split('#').pop() || targetBase;
        for (const node of this.currentPayload.nodes) {
          if (seen.has(node.id)) continue;
          const nPath = (node.path || '').replace(/\\/g, '/').toLowerCase();
          const nTitle = (node.title || '').toLowerCase();
          if (
            nPath.includes(targetBase) ||
            nPath.includes(targetSym) ||
            (nTitle && (target.includes(nTitle) || nTitle.includes(targetSym)))
          ) {
            seen.add(node.id);
            results.push(node);
            if (results.length >= 12) break;
          }
        }
      }
    }

    return results;
  }
}

function bootstrap() {
  const app = new GraphViewApp();
  (window as any).__graphApp = app;
  app.init();
}

if (document.readyState === 'loading') {
  document.addEventListener('DOMContentLoaded', bootstrap);
} else {
  bootstrap();
}
