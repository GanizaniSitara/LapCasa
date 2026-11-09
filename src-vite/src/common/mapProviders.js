const GLOBAL_THEMES = [
  {
    maxZoom: 19,
    layers: [
      { url: 'https://tile.openstreetmap.org/{z}/{x}/{y}.png', attribution: 'OpenStreetMap' },
    ],
  },
  {
    maxZoom: 17,
    layers: [
      { url: 'https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile/{z}/{y}/{x}', attribution: 'Powered by Esri' },
    ],
  },
];

function tiandituUrl(layer, token) {
  return `https://t{s}.tianditu.gov.cn/${layer}_w/wmts?SERVICE=WMTS&REQUEST=GetTile&VERSION=1.0.0&LAYER=${layer.slice(0, 3)}&STYLE=default&TILEMATRIXSET=w&TILEMATRIX={z}&TILEROW={y}&TILECOL={x}&FORMAT=tiles&tk=${encodeURIComponent(token)}`;
}

export function getMapThemes(provider, tiandituToken) {
  const token = String(tiandituToken || '').trim();
  if (provider === 'osm') return [GLOBAL_THEMES[0], GLOBAL_THEMES[0]];
  if (provider === 'esri') return [GLOBAL_THEMES[1], GLOBAL_THEMES[1]];
  if (provider !== 'tianditu' || !token) return [{ offline: true, maxZoom: 19, layers: [] }];

  const attribution = 'Tianditu';
  return [
    {
      maxZoom: 18,
      layers: [
        { url: tiandituUrl('vec', token), attribution, subdomains: '01234567' },
        { url: tiandituUrl('cva', token), attribution, subdomains: '01234567' },
      ],
    },
    {
      maxZoom: 18,
      layers: [
        { url: tiandituUrl('img', token), attribution, subdomains: '01234567' },
        { url: tiandituUrl('cia', token), attribution, subdomains: '01234567' },
      ],
    },
  ];
}

export function getMapTheme(provider, tiandituToken, themeIndex, localMapPath = '') {
  const themes = getMapThemes(provider, tiandituToken);
  const theme = themes[Number(themeIndex) === 1 ? 1 : 0] || themes[0];
  return theme.offline ? { ...theme, localMapPath } : theme;
}

export function createTileLayerGroup(L, theme) {
  if (theme.offline) {
    const overview = L.geoJSON(worldOverview, {
      style: { color: '#7e8985', weight: 0.7, fillColor: '#b7c6b4', fillOpacity: 1 },
      interactive: false,
      attribution: 'Natural Earth · offline overview',
    });
    if (!theme.localMapPath) return { layer: overview, tileLayers: [] };
    const LocalTiles = L.GridLayer.extend({
      createTile(coords, done) {
        const canvas = document.createElement('canvas');
        canvas.width = canvas.height = 256;
        invoke('local_map_tile', { path: theme.localMapPath, z: coords.z, x: coords.x, y: coords.y })
          .then(source => {
            if (!source) { done(null, canvas); return; }
            const image = new Image();
            image.onload = () => { canvas.getContext('2d').drawImage(image, 0, 0, 256, 256); done(null, canvas); };
            image.onerror = () => done(new Error('Local map tile could not be decoded'), canvas);
            image.src = source;
          }).catch(error => done(new Error(String(error)), canvas));
        return canvas;
      },
    });
    const tiles = new LocalTiles({ maxZoom: 19, noWrap: true });
    tiles.on('add', async () => {
      try {
        const info = await invoke('local_map_info', { path: theme.localMapPath });
        if (!tiles._map) return;
        // Pack metadata is untrusted text, never provider-supplied HTML.
        const escaped = String(info.attribution || info.name || 'Local map')
          .replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]);
        tiles.options.attribution = escaped;
        tiles._map.attributionControl.addAttribution(escaped);
        tiles.options.minNativeZoom = info.minZoom;
        tiles.options.maxNativeZoom = info.maxZoom;
        tiles.redraw();
      } catch (error) { tiles.fire('tileerror', { error }); }
    });
    return { layer: L.layerGroup([overview, tiles]), tileLayers: [tiles] };
  }
  const layers = theme.layers.map(({ url, attribution, subdomains }) => L.tileLayer(url, {
    attribution,
    maxZoom: theme.maxZoom,
    ...(subdomains ? { subdomains } : {}),
  }));
  return {
    layer: layers.length === 1 ? layers[0] : L.layerGroup(layers),
    tileLayers: layers,
  };
}
import { invoke } from '@tauri-apps/api/core';
import worldOverview from '@/assets/world-overview.json';

