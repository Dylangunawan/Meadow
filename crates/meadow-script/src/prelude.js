// Meadow's WorldPainter-compatible scripting API. Mirrors WorldPainter's `wp` fluent
// operations and the Java-style World2/Dimension methods scripts call, on top of the native
// bridge object `__m` provided by Meadow.
(function (global) {
  'use strict';
  var M = global.__m;

  function err(msg) { throw new Error(msg); }

  // ---- Terrain enum ------------------------------------------------------------------------
  var terrainInfo = JSON.parse(M.terrains());
  var Terrain = { VALUES: [], __byIndex: [] };
  terrainInfo.forEach(function (t) {
    var o = Object.freeze({
      name: t.name, index: t.index, __terrain: t.index,
      getName: function () { return t.name; },
      ordinal: function () { return t.index; },
      toString: function () { return t.name; }
    });
    Terrain[t.key] = o;
    Terrain.VALUES.push(o);
    Terrain.__byIndex[t.index] = o;
  });
  Terrain.values = function () { return Terrain.VALUES.slice(); };
  Terrain.getCustomTerrain = function (i) { return Terrain.__byIndex[M.customSlotIndex(i)]; };
  function terrainIndex(t) {
    if (t === null || t === undefined) err('terrain may not be null');
    if (typeof t === 'number') return t;
    if (t.__terrain !== undefined) return t.__terrain;
    err('not a terrain: ' + t);
  }

  // ---- Layers ------------------------------------------------------------------------------
  function Layer(info, worldId) {
    this.__layer = info.id; this.name = info.name; this.dataSize = info.dataSize; this.__world = worldId;
  }
  Layer.prototype.getName = function () { return this.name; };
  Layer.prototype.getId = function () { return String(this.__layer); };
  Layer.prototype.getDataSize = function () { return this.dataSize; };
  Layer.prototype.toString = function () { return this.name; };
  function layerOf(worldId, name) {
    var j = M.layerByName(worldId === undefined ? -1 : worldId, String(name));
    if (!j) err('No layer named "' + name + '"');
    return new Layer(JSON.parse(j), worldId);
  }
  function layerId(l) {
    if (l === null || l === undefined) err('layer may not be null');
    if (l.__layer !== undefined) return l.__layer;
    if (typeof l === 'string') return layerOf(-1, l).__layer;
    err('not a layer: ' + l);
  }

  // ---- Height maps -------------------------------------------------------------------------
  function HeightMap(id) { this.__hm = id; }
  HeightMap.prototype.getWidth = function () { return M.hmWidth(this.__hm); };
  HeightMap.prototype.getHeight = function (x, y) {
    if (x === undefined) return M.hmHeight(this.__hm);
    return M.hmGet(this.__hm, x, y);
  };
  HeightMap.prototype.getColour = function (x, y) { return M.hmColour(this.__hm, x, y); };
  HeightMap.prototype.getColor = HeightMap.prototype.getColour;
  HeightMap.prototype.getExtent = function () {
    return { x: 0, y: 0, width: M.hmWidth(this.__hm), height: M.hmHeight(this.__hm) };
  };

  // ---- World / Dimension -------------------------------------------------------------------
  function Dimension(w, d) { this.__w = w; this.__d = d; }
  var D = Dimension.prototype;
  D.getHeightAt = function (x, y) { return M.getHeight(this.__w, this.__d, x, y); };
  D.getIntHeightAt = function (x, y) { return M.getIntHeight(this.__w, this.__d, x, y); };
  D.getRawHeightAt = function (x, y) { return Math.round(M.getHeight(this.__w, this.__d, x, y) * 256); };
  D.setHeightAt = function (x, y, h) { M.setHeight(this.__w, this.__d, x, y, h); };
  D.getTerrainAt = function (x, y) {
    var i = M.getTerrain(this.__w, this.__d, x, y);
    return i < 0 ? null : Terrain.__byIndex[i];
  };
  D.setTerrainAt = function (x, y, t) { M.setTerrain(this.__w, this.__d, x, y, terrainIndex(t)); };
  D.getWaterLevelAt = function (x, y) { return M.getWater(this.__w, this.__d, x, y); };
  D.setWaterLevelAt = function (x, y, l) { M.setWater(this.__w, this.__d, x, y, l); };
  D.getLayerValueAt = function (l, x, y) { return M.getLayer(this.__w, this.__d, layerId(l), x, y); };
  D.setLayerValueAt = function (l, x, y, v) { M.setLayer(this.__w, this.__d, layerId(l), x, y, v); };
  D.getBitLayerValueAt = function (l, x, y) { return M.getLayer(this.__w, this.__d, layerId(l), x, y) !== 0; };
  D.setBitLayerValueAt = function (l, x, y, v) { M.setLayer(this.__w, this.__d, layerId(l), x, y, v ? 1 : 0); };
  D.clearLayerData = function (l) { M.clearLayer(this.__w, this.__d, layerId(l)); };
  D.getSlope = function (x, y) { return M.getSlope(this.__w, this.__d, x, y); };
  D.getBiomeAt = function (x, y) { return M.getLayer(this.__w, this.__d, 11, x, y); };
  D.setBiomeAt = function (x, y, b) { M.setLayer(this.__w, this.__d, 11, x, y, b); };
  D.isTilePresent = function (tx, ty) { return M.tilePresent(this.__w, this.__d, tx, ty); };
  D.getTileCount = function () { return M.tileCount(this.__w, this.__d); };
  function bounds(d) { return JSON.parse(M.bounds(d.__w, d.__d)); }
  D.getLowestX = function () { return bounds(this)[0]; };
  D.getLowestY = function () { return bounds(this)[1]; };
  D.getHighestX = function () { return bounds(this)[2]; };
  D.getHighestY = function () { return bounds(this)[3]; };
  D.getWidth = function () { var b = bounds(this); return b[2] - b[0] + 1; };
  D.getHeight = function () { var b = bounds(this); return b[3] - b[1] + 1; };
  D.getMinHeight = function () { return M.minHeight(this.__w, this.__d); };
  D.getMaxHeight = function () { return M.maxHeight(this.__w, this.__d); };
  D.getMinimumLayers = D.getMinHeight;
  D.getSeed = function () { return M.seed(this.__w); };
  D.getDim = function () { return this.__d; };
  D.getName = function () { return ['Surface', 'Nether', 'End'][this.__d] || 'Dimension'; };
  D.getWorld = function () { return new World(this.__w); };
  D.getTileCoords = function () { return JSON.parse(M.tileCoords(this.__w, this.__d)).map(function (c) { return { x: c[0], y: c[1] }; }); };
  D.isDirty = function () { return true; };
  D.setDirty = function () {};

  function World(id) { this.__id = id; }
  var W = World.prototype;
  W.getName = function () { return M.worldName(this.__id); };
  W.setName = function (n) { M.setWorldName(this.__id, String(n)); };
  W.getDimension = function (d) {
    var idx = (d === undefined) ? 0 : (typeof d === 'object' && d.dim !== undefined ? d.dim : d);
    return M.dimExists(this.__id, idx) ? new Dimension(this.__id, idx) : null;
  };
  W.getDimensions = function () {
    var out = []; for (var i = 0; i < 3; i++) if (M.dimExists(this.__id, i)) out.push(new Dimension(this.__id, i));
    return out;
  };
  W.getMaxHeight = function () { return M.maxHeight(this.__id, 0); };
  W.getMinHeight = function () { return M.minHeight(this.__id, 0); };
  W.getSeed = function () { return M.seed(this.__id); };
  W.getSpawnPoint = function () { var s = JSON.parse(M.spawn(this.__id)); return { x: s[0], y: s[1] }; };
  W.setSpawnPoint = function (p) { M.setSpawn(this.__id, p.x, p.y); };
  W.getLayer = function (name) { return layerOf(this.__id, name); };
  W.toString = function () { return 'World[' + this.getName() + ']'; };

  // ---- Fluent operations -------------------------------------------------------------------
  function op(fields, goFn) {
    var o = { __goCalled: false };
    Object.keys(fields).forEach(function (k) { o[k] = fields[k]; });
    o.go = function () { o.__goCalled = true; return goFn.call(o); };
    return o;
  }

  function mappingOp(init) {
    var s = { world: null, dim: 0, heightMap: null, layer: null, terrain: -1, mode: 'set',
              mapping: [], colours: [], scale: 100, offsetX: 0, offsetY: 0, filter: null,
              pendingFrom: null, pendingColour: null };
    Object.keys(init).forEach(function (k) { s[k] = init[k]; });
    var o = {};
    function store(v) {
      if (s.pendingColour !== null) { s.colours.push([s.pendingColour, v]); s.pendingColour = null; }
      else if (s.pendingFrom !== null) { s.mapping.push([s.pendingFrom[0], s.pendingFrom[1], v, v]); }
      else err('call fromLevel(s) or fromColour first');
    }
    o.toWorld = function (w) { s.world = w.__id; return o; };
    o.world = o.toWorld;
    o.heightMap = function (h) { s.heightMap = h.__hm; return o; };
    o.applyToLayer = function (l) { s.layer = layerId(l); if (s.mode === 'terrain') s.mode = 'set'; return o; };
    o.applyToTerrain = function () { s.mode = 'terrain'; return o; };
    o.applyToSurface = function () { s.dim = 0; return o; };
    o.applyToNether = function () { s.dim = 1; return o; };
    o.applyToEnd = function () { s.dim = 2; return o; };
    o.applyToSurfaceCeiling = function () { err('ceiling dimensions are not supported by Meadow'); };
    o.applyToNetherCeiling = o.applyToSurfaceCeiling;
    o.applyToEndCeiling = o.applyToSurfaceCeiling;
    o.fromLevel = function (l) { s.pendingFrom = [l, l]; s.pendingColour = null; return o; };
    o.fromLevels = function (a, b) { s.pendingFrom = [a, b]; s.pendingColour = null; return o; };
    o.fromColour = function (a, r, g, b) {
      if (b === undefined) { b = g; g = r; r = a; a = 255; }
      s.pendingColour = ((a & 255) << 24 | (r & 255) << 16 | (g & 255) << 8 | (b & 255)) >>> 0; return o;
    };
    o.fromColor = o.fromColour;
    o.toLevel = function (l) { store(l); s.layerValue = l; return o; };
    o.toTerrain = function (t) { store(terrainIndex(t)); return o; };
    o.toLevels = function (a, b) {
      if (s.pendingFrom === null) err('call fromLevels first');
      s.mapping.push([s.pendingFrom[0], s.pendingFrom[1], a, b]); return o;
    };
    o.setAlways = function () { s.mode = (s.mode === 'terrain') ? 'terrain' : 'set'; return o; };
    o.setWhenLower = function () { s.mode = 'setWhenLower'; return o; };
    o.setWhenHigher = function () { s.mode = 'setWhenHigher'; return o; };
    o.scale = function (p) { s.scale = p; return o; };
    o.shift = function (x, y) { s.offsetX = x; s.offsetY = y; return o; };
    o.withFilter = function (f) { s.filter = (f === null || f === undefined) ? null : f.__filter; return o; };
    o.go = function () {
      if (s.world === null) {
        if (global.world && global.world.__id !== undefined) s.world = global.world.__id; else err('world not specified');
      }
      if (global.__debugMapping) print(JSON.stringify(s));
      M.mapping(JSON.stringify(s)); return null;
    };
    return o;
  }

  function filterOp() {
    var s = { above: null, below: null, feather: false, only: [], except: [], aboveDeg: null, belowDeg: null, selection: null, last: null };
    var o = {};
    function cond(list, c) { list.push(c); s.last = c; return o; }
    o.aboveLevel = function (l) { s.above = l; return o; };
    o.belowLevel = function (l) { s.below = l; return o; };
    o.feather = function () { s.feather = true; return o; };
    o.onlyOnTerrain = function (t) { return cond(s.only, { terrain: terrainIndex(t) }); };
    o.onlyOnLayer = function (l) { return cond(s.only, { layer: layerId(l), value: null, cmp: 'any' }); };
    o.onlyOnBiome = function (b) { return cond(s.only, { biome: b }); };
    o.onlyOnAutoBiome = function (b) { return cond(s.only, { autoBiome: b }); };
    o.onlyOnAutoBiomes = function () { return cond(s.only, { autoBiomes: true }); };
    o.onlyOnWater = function () { return cond(s.only, { water: true }); };
    o.onlyOnLand = function () { return cond(s.only, { water: false }); };
    o.exceptOnTerrain = function (t) { return cond(s.except, { terrain: terrainIndex(t) }); };
    o.exceptOnLayer = function (l) { return cond(s.except, { layer: layerId(l), value: null, cmp: 'any' }); };
    o.exceptOnBiome = function (b) { return cond(s.except, { biome: b }); };
    o.exceptOnAutoBiome = function (b) { return cond(s.except, { autoBiome: b }); };
    o.exceptOnAutoBiomes = function () { return cond(s.except, { autoBiomes: true }); };
    o.exceptOnWater = function () { return cond(s.except, { water: true }); };
    o.exceptOnLand = function () { return cond(s.except, { water: false }); };
    o.withValue = function (v) { if (!s.last || s.last.layer === undefined) err('withValue() must follow a layer condition'); s.last.value = v; s.last.cmp = 'eq'; return o; };
    o.orHigher = function () { if (!s.last || s.last.value === null) err('orHigher() must follow withValue()'); s.last.cmp = 'ge'; return o; };
    o.orLower = function () { if (!s.last || s.last.value === null) err('orLower() must follow withValue()'); s.last.cmp = 'le'; return o; };
    o.aboveDegrees = function (d) { s.aboveDeg = d; return o; };
    o.belowDegrees = function (d) { s.belowDeg = d; return o; };
    o.inSelection = function () { s.selection = true; return o; };
    o.outsideSelection = function () { s.selection = false; return o; };
    o.go = function () { return { __filter: M.createFilter(JSON.stringify(s)) }; };
    return o;
  }

  var wp = {
    getVersion: function () { return M.version(); },
    getWorld: function () {
      var file = null;
      return op({ fromFile: function (f) { file = String(f); return this; } }, function () {
        if (file === null) err('fromFile() not called');
        return new World(M.loadWorld(file));
      });
    },
    createWorld: function () {
      var s = { heightMap: null, scale: 100, offsetX: 0, offsetY: 0, imageLow: null, imageHigh: null,
                worldLow: null, worldHigh: null, waterLevel: 62, lower: null, upper: null, format: null };
      var o = op({}, function () { return new World(M.createWorld(JSON.stringify(s))); });
      o.fromHeightMap = function (h) { s.heightMap = h.__hm; return o; };
      o.heightMap = o.fromHeightMap;
      o.scale = function (p) { s.scale = p; return o; };
      o.shift = function (x, y) { s.offsetX = x; s.offsetY = y; return o; };
      o.fromLevels = function (a, b) { s.imageLow = a; s.imageHigh = b; return o; };
      o.toLevels = function (a, b) { s.worldLow = a; s.worldHigh = b; return o; };
      o.withWaterLevel = function (l) { s.waterLevel = l; return o; };
      o.waterLevel = o.withWaterLevel;
      o.withMapFormat = function (f) { s.format = f ? String(f) : null; return o; };
      o.withLowerBuildLimit = function (l) { s.lower = l; return o; };
      o.withUpperBuildLimit = function (l) { s.upper = l; return o; };
      return o;
    },
    exportWorld: function (w) {
      var s = { world: w ? w.__id : null, dir: null, format: null };
      var o = op({}, function () {
        if (s.world === null) err('world not specified');
        if (s.dir === null) err('toDirectory() not called');
        print(M.exportWorld(JSON.stringify(s)));
        return null;
      });
      o.toDirectory = function (d) { s.dir = String(d); return o; };
      o.world = function (w2) { s.world = w2.__id; return o; };
      o.withMapFormat = function (f) { s.format = f ? String(f) : null; return o; };
      return o;
    },
    saveWorld: function (w) {
      var s = { world: w ? w.__id : null, file: null };
      var o = op({}, function () {
        if (s.world === null) err('world not specified');
        if (s.file === null) err('toFile() not called');
        M.saveWorld(s.world, s.file); return null;
      });
      o.toFile = function (f) { s.file = String(f); return o; };
      o.world = function (w2) { s.world = w2.__id; return o; };
      return o;
    },
    mergeWorld: function () { err('Merging into existing Minecraft maps is not supported by Meadow yet'); },
    getHeightMap: function () {
      var s = { file: null, channel: 'grey' };
      var o = op({}, function () {
        if (s.file === null) err('fromFile() not called');
        return new HeightMap(M.loadHeightMap(s.file, s.channel));
      });
      o.fromFile = function (f) { s.file = String(f); return o; };
      o.selectRedChannel = function () { s.channel = 'red'; return o; };
      o.selectGreenChannel = function () { s.channel = 'green'; return o; };
      o.selectBlueChannel = function () { s.channel = 'blue'; return o; };
      return o;
    },
    getLayer: function () {
      var s = { world: undefined, name: null, file: null };
      var o = op({}, function () {
        if (s.file !== null) err('WorldPainter .layer files cannot be read by Meadow; recreate the layer and use withName("' + s.file + '")');
        if (s.name === null) err('withName() not called');
        return layerOf(s.world === undefined ? (global.world && global.world.__id !== undefined ? global.world.__id : -1) : s.world, s.name);
      });
      o.fromFile = function (f) { s.file = String(f); return o; };
      o.fromWorld = function (w) { s.world = w.__id; return o; };
      o.withName = function (n) { s.name = String(n); return o; };
      o.name = o.withName;
      return o;
    },
    getTerrain: function () {
      return op({ fromFile: function () { return this; } }, function () {
        err('WorldPainter .terrain files cannot be read by Meadow; define custom terrains in Meadow instead');
      });
    },
    installCustomTerrain: function () { err('installCustomTerrain is not supported by Meadow; define custom terrains in Meadow instead'); },
    applyHeightMap: function (h) { return mappingOp(h ? { heightMap: h.__hm } : {}); },
    applyLayer: function (l) { return mappingOp({ layer: layerId(l), layerValue: M.layerDefaultValue(layerId(l)), direct: true }); },
    applyTerrain: function (t) { return mappingOp({ terrain: terrainIndex(t), mode: 'terrain', direct: true }); },
    createFilter: function () { return filterOp(); },
    getMapFormat: function () {
      var id = null;
      return op({ withId: function (i) { id = String(i); return this; } }, function () { return id; });
    },
    getBiomeId: function () {
      var name = null;
      return op({ withName: function (n) { name = String(n); return this; } }, function () {
        var v = M.biomeId(name); if (v < 0) err('Unknown biome ' + name); return v;
      });
    },
    isInterrupted: function () { return false; },
    checkForInterrupt: function () {}
  };
  // Bean-style property access used by Nashorn scripts (wp.version, wp.world, ...)
  Object.defineProperty(wp, 'version', { get: function () { return M.version(); } });
  Object.defineProperty(wp, 'world', { get: function () { return wp.getWorld(); } });
  Object.defineProperty(wp, 'layer', { get: function () { return wp.getLayer(); } });
  Object.defineProperty(wp, 'heightMap', { get: function () { return wp.getHeightMap(); } });
  Object.defineProperty(wp, 'terrain', { get: function () { return wp.getTerrain(); } });
  Object.defineProperty(wp, 'mapFormat', { get: function () { return wp.getMapFormat(); } });
  Object.defineProperty(wp, 'biomeId', { get: function () { return wp.getBiomeId(); } });

  // ---- Nashorn / Java compatibility --------------------------------------------------------
  global.print = function () { M.print(Array.prototype.map.call(arguments, String).join(' ')); };
  global.exit = function (code) { throw { __exit: code || 0 }; };
  global.quit = global.exit;
  var Constants = { DIM_NORMAL: 0, DIM_NETHER: 1, DIM_END: 2, DIM_NORMAL_CEILING: -1, DIM_NETHER_CEILING: -2, DIM_END_CEILING: -3,
                    TILE_SIZE: 128, TILE_SIZE_BITS: 7, TILE_SIZE_MASK: 127 };
  var layerClasses = {};
  ['Frost', 'Caves', 'Caverns', 'Chasms', 'DeciduousForest', 'PineForest', 'SwampLand', 'Jungle', 'Void', 'Resources',
   'Biome', 'Populate', 'ReadOnly', 'Annotations', 'FloodWithLava'].forEach(function (n) {
    var lookup = { DeciduousForest: 'Deciduous', PineForest: 'Pine', SwampLand: 'Swamp', ReadOnly: 'Read Only', FloodWithLava: 'Lava' }[n] || n;
    var cls = {};
    Object.defineProperty(cls, 'INSTANCE', { get: function () { return layerOf(-1, lookup); } });
    layerClasses[n] = cls;
  });
  var System = { out: { println: function (s) { print(s); }, print: function (s) { M.printRaw(String(s)); } },
                 err: { println: function (s) { print(s); } },
                 currentTimeMillis: function () { return Date.now(); },
                 nanoTime: function () { return Date.now() * 1e6; },
                 getProperty: function () { return null; } };
  var JavaFile = function (a, b) { this.path = b === undefined ? String(a) : String(a) + '/' + String(b); };
  JavaFile.prototype.getPath = function () { return this.path; };
  JavaFile.prototype.getName = function () { return this.path.split(/[\\/]/).pop(); };
  JavaFile.prototype.toString = function () { return this.path; };
  JavaFile.prototype.exists = function () { return M.fileExists(this.path); };
  var classes = {
    'org.pepsoft.worldpainter.Terrain': Terrain,
    'org.pepsoft.worldpainter.Constants': Constants,
    'java.lang.Math': Math,
    'java.lang.System': System,
    'java.io.File': JavaFile,
    'java.lang.Integer': { MAX_VALUE: 2147483647, MIN_VALUE: -2147483648, parseInt: function (s) { return parseInt(s, 10); } },
    'java.lang.String': String
  };
  Object.keys(layerClasses).forEach(function (n) { classes['org.pepsoft.worldpainter.layers.' + n] = layerClasses[n]; });
  global.Java = {
    type: function (name) {
      if (classes[name]) return classes[name];
      err('Java class ' + name + ' is not available in Meadow');
    },
    from: function (a) { return Array.prototype.slice.call(a); },
    to: function (a) { return a; }
  };
  global.org = { pepsoft: { worldpainter: { Terrain: Terrain, Constants: Constants, layers: layerClasses } } };
  global.java = { lang: { Math: Math, System: System }, io: { File: JavaFile } };
  global.Packages = { org: global.org, java: global.java };
  global.Terrain = Terrain;
  global.Constants = Constants;
  global.wp = wp;
  global.__World = World;
  global.__Dimension = Dimension;
  global.__HeightMap = HeightMap;
})(this);
