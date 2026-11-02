// Courier script sandbox prelude.
//
// Builds the `pm` API on top of a JSON input supplied by the host and
// collects results into `__state`, which the host reads back as JSON.
// Native helpers provided by the host:
//   __courier_send(json) -> json        pm.sendRequest
//   __courier_jar(json) -> json         pm.cookies.jar()
//   __courier_hash(alg, data, key) -> hex
//   __courier_b64(mode, data) -> string
//   __courier_dynamic(name) -> string|undefined
//   __courier_uuid() -> string

var __in = JSON.parse(__courier_input);
var __state = {
  tests: [],
  console: [],
  nextSet: false,
  next: null,
  skip: false,
  visualizer: null,
  pending: 0,
};

// ------------------------------------------------------------------ utils

function __inspect(v) {
  if (v === undefined) return 'undefined';
  if (typeof v === 'string') return "'" + v + "'";
  if (typeof v === 'function') return '[Function' + (v.name ? ': ' + v.name : '') + ']';
  if (v instanceof Error) return v.name + ': ' + v.message;
  try {
    var s = JSON.stringify(v);
    return s === undefined ? String(v) : s;
  } catch (e) {
    return String(v);
  }
}

function __fmt(args) {
  return Array.prototype.map
    .call(args, function (a) {
      return typeof a === 'string' ? a : __inspect(a);
    })
    .join(' ');
}

function __toStr(v) {
  if (v === undefined || v === null) return '';
  if (typeof v === 'string') return v;
  if (typeof v === 'object') return JSON.stringify(v);
  return String(v);
}

function __deepEqual(a, b) {
  if (a === b) return true;
  if (typeof a === 'number' && typeof b === 'number' && isNaN(a) && isNaN(b)) return true;
  if (a === null || b === null || typeof a !== 'object' || typeof b !== 'object') return false;
  if (Array.isArray(a) !== Array.isArray(b)) return false;
  if (a instanceof Date && b instanceof Date) return a.getTime() === b.getTime();
  var ka = Object.keys(a), kb = Object.keys(b);
  if (ka.length !== kb.length) return false;
  for (var i = 0; i < ka.length; i++) {
    if (!Object.prototype.hasOwnProperty.call(b, ka[i])) return false;
    if (!__deepEqual(a[ka[i]], b[ka[i]])) return false;
  }
  return true;
}

function __getPath(obj, path) {
  var parts = String(path).replace(/\[(\w+)\]/g, '.$1').split('.').filter(function (p) { return p !== ''; });
  var cur = obj;
  for (var i = 0; i < parts.length; i++) {
    if (cur === null || cur === undefined || !(parts[i] in Object(cur))) return { exists: false };
    cur = cur[parts[i]];
  }
  return { exists: true, value: cur };
}

// ------------------------------------------------------------------ console & timers

var console = {};
['log', 'info', 'warn', 'error', 'debug'].forEach(function (level) {
  console[level] = function () {
    __state.console.push({ level: level, message: __fmt(arguments) });
  };
});

var __timers = [];
var __timerSeq = 0;
function setTimeout(fn, ms) {
  var id = ++__timerSeq;
  __timers.push({ id: id, fn: fn, at: Number(ms) || 0, args: Array.prototype.slice.call(arguments, 2) });
  return id;
}
function clearTimeout(id) {
  __timers = __timers.filter(function (t) { return t.id !== id; });
}
var setInterval = function (fn, ms) {
  console.warn('setInterval is not supported in Courier scripts; running once.');
  return setTimeout(fn, ms);
};
var clearInterval = clearTimeout;
function setImmediate(fn) { return setTimeout(fn, 0); }

function __courier_drain_timers() {
  if (!__timers.length) return false;
  __timers.sort(function (a, b) { return a.at - b.at || a.id - b.id; });
  var t = __timers.shift();
  try {
    t.fn.apply(null, t.args);
  } catch (e) {
    __state.console.push({ level: 'error', message: 'Uncaught in timer: ' + __inspect(e) });
  }
  return true;
}

// ------------------------------------------------------------------ encoding & crypto

function btoa(s) { return __courier_b64('encode', String(s)); }
function atob(s) { return __courier_b64('decode', String(s)); }

function __WordArray(hex) { this.hex = hex; }
__WordArray.prototype.toString = function (enc) {
  if (enc && enc.__kind === 'base64') return __courier_b64('hex2b64', this.hex);
  return this.hex;
};
var CryptoJS = {
  enc: {
    Hex: { __kind: 'hex', stringify: function (w) { return w.hex; } },
    Base64: {
      __kind: 'base64',
      stringify: function (w) { return w.__str !== undefined ? btoa(w.__str) : __courier_b64('hex2b64', w.hex); },
      parse: function (s) { var o = new __WordArray(__courier_b64('b642hex', s)); o.__str = atob(s); return o; },
    },
    Utf8: {
      __kind: 'utf8',
      parse: function (s) { var o = new __WordArray(__courier_hash('hex', String(s), null)); o.__str = String(s); return o; },
      stringify: function (w) { return w.__str !== undefined ? w.__str : ''; },
    },
  },
};
function __data(d) { return d instanceof __WordArray && d.__str !== undefined ? d.__str : String(d); }
[['MD5', 'md5'], ['SHA1', 'sha1'], ['SHA256', 'sha256'], ['SHA512', 'sha512']].forEach(function (p) {
  CryptoJS[p[0]] = function (d) { return new __WordArray(__courier_hash(p[1], __data(d), null)); };
  CryptoJS['Hmac' + p[0]] = function (d, key) { return new __WordArray(__courier_hash(p[1], __data(d), __data(key))); };
});

// ------------------------------------------------------------------ xml2Json

function xml2Json(text) {
  var pos = 0, s = String(text);
  function ws() { while (pos < s.length && /\s/.test(s[pos])) pos++; }
  function skipMisc() {
    for (;;) {
      ws();
      if (s.startsWith('<?', pos)) pos = s.indexOf('?>', pos) + 2;
      else if (s.startsWith('<!--', pos)) pos = s.indexOf('-->', pos) + 3;
      else if (s.startsWith('<!', pos)) pos = s.indexOf('>', pos) + 1;
      else return;
    }
  }
  function decode(t) {
    return t.replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/&quot;/g, '"').replace(/&apos;/g, "'").replace(/&amp;/g, '&');
  }
  function element() {
    pos++; // <
    var name = /^[^\s/>]+/.exec(s.slice(pos))[0];
    pos += name.length;
    var node = {}, hasAttr = false;
    var attrRe = /^\s*([^\s=/>]+)\s*=\s*("([^"]*)"|'([^']*)')/;
    var m;
    while ((m = attrRe.exec(s.slice(pos)))) {
      node['$'] = node['$'] || {};
      node['$'][m[1]] = decode(m[3] !== undefined ? m[3] : m[4]);
      hasAttr = true;
      pos += m[0].length;
    }
    ws();
    if (s.startsWith('/>', pos)) { pos += 2; return hasAttr ? node : ''; }
    pos++; // >
    var textParts = [], children = false;
    for (;;) {
      if (s.startsWith('</', pos)) { pos = s.indexOf('>', pos) + 1; break; }
      if (s.startsWith('<!--', pos)) { pos = s.indexOf('-->', pos) + 3; continue; }
      if (s.startsWith('<![CDATA[', pos)) {
        var end = s.indexOf(']]>', pos);
        textParts.push(s.slice(pos + 9, end));
        pos = end + 3;
        continue;
      }
      if (s[pos] === '<') {
        var childName = /^<([^\s/>]+)/.exec(s.slice(pos))[1];
        var child = element();
        children = true;
        if (node[childName] === undefined) node[childName] = child;
        else if (Array.isArray(node[childName])) node[childName].push(child);
        else node[childName] = [node[childName], child];
        continue;
      }
      if (pos >= s.length) throw new Error('xml2Json: unexpected end of input');
      var next = s.indexOf('<', pos);
      textParts.push(decode(s.slice(pos, next)));
      pos = next;
    }
    var txt = textParts.join('').trim();
    if (!children && !hasAttr) return txt;
    if (txt) node['_'] = txt;
    return node;
  }
  skipMisc();
  var rootName = /^<([^\s/>]+)/.exec(s.slice(pos));
  if (!rootName) throw new Error('xml2Json: no root element');
  var out = {};
  out[rootName[1]] = element();
  return out;
}

// ------------------------------------------------------------------ assertions (BDD style)

function AssertionError(message, expected, actual) {
  this.name = 'AssertionError';
  this.message = message;
  this.expected = expected;
  this.actual = actual;
}
AssertionError.prototype = Object.create(Error.prototype);
AssertionError.prototype.constructor = AssertionError;

function Assertion(obj, msg) {
  this._obj = obj;
  this._msg = msg;
  this._negate = false;
  this._deep = false;
  this._nested = false;
  this._own = false;
  this._any = false;
}

Assertion.prototype._assert = function (ok, message, negMessage, expected, actual) {
  if (this._negate) ok = !ok;
  if (!ok) {
    var m = this._negate ? negMessage : message;
    throw new AssertionError((this._msg ? this._msg + ': ' : '') + m, expected, actual);
  }
  return this;
};

function __chain(name, fn) {
  Object.defineProperty(Assertion.prototype, name, { get: fn, configurable: true });
}
['to', 'be', 'been', 'is', 'that', 'which', 'and', 'has', 'have', 'with', 'at', 'of', 'same', 'but', 'does', 'still', 'also'].forEach(
  function (w) { __chain(w, function () { return this; }); }
);
__chain('not', function () { this._negate = !this._negate; return this; });
__chain('deep', function () { this._deep = true; return this; });
__chain('nested', function () { this._nested = true; return this; });
__chain('own', function () { this._own = true; return this; });
__chain('any', function () { this._any = true; return this; });
__chain('all', function () { this._any = false; return this; });

function __prop(name, check, msg) {
  __chain(name, function () {
    var o = this._obj;
    return this._assert(check(o), 'expected ' + __inspect(o) + ' to ' + msg, 'expected ' + __inspect(o) + ' not to ' + msg);
  });
}
__prop('ok', function (o) { return !!o; }, 'be truthy');
__prop('true', function (o) { return o === true; }, 'be true');
__prop('false', function (o) { return o === false; }, 'be false');
__prop('null', function (o) { return o === null; }, 'be null');
__prop('undefined', function (o) { return o === undefined; }, 'be undefined');
__prop('NaN', function (o) { return typeof o === 'number' && isNaN(o); }, 'be NaN');
__prop('exist', function (o) { return o !== null && o !== undefined; }, 'exist');
__prop('exists', function (o) { return o !== null && o !== undefined; }, 'exist');
__prop('finite', function (o) { return typeof o === 'number' && isFinite(o); }, 'be a finite number');
__prop('empty', function (o) {
  if (typeof o === 'string' || Array.isArray(o)) return o.length === 0;
  if (o && typeof o === 'object') return Object.keys(o).length === 0;
  return false;
}, 'be empty');

var A = Assertion.prototype;

A.equal = A.equals = A.eq = function (v) {
  var ok = this._deep ? __deepEqual(this._obj, v) : this._obj === v;
  return this._assert(ok, 'expected ' + __inspect(this._obj) + ' to ' + (this._deep ? 'deeply ' : '') + 'equal ' + __inspect(v),
    'expected ' + __inspect(this._obj) + ' to not ' + (this._deep ? 'deeply ' : '') + 'equal ' + __inspect(v), v, this._obj);
};
A.eql = A.eqls = function (v) {
  return this._assert(__deepEqual(this._obj, v), 'expected ' + __inspect(this._obj) + ' to deeply equal ' + __inspect(v),
    'expected ' + __inspect(this._obj) + ' to not deeply equal ' + __inspect(v), v, this._obj);
};
function __typeImpl(type) {
  var o = this._obj;
  var actual = o === null ? 'null' : Array.isArray(o) ? 'array' : o instanceof Date ? 'date' : o instanceof RegExp ? 'regexp' : typeof o;
  var t = String(type).toLowerCase();
  return this._assert(actual === t, 'expected ' + __inspect(o) + ' to be a' + (/^[aeiou]/.test(t) ? 'n ' : ' ') + t,
    'expected ' + __inspect(o) + ' not to be a' + (/^[aeiou]/.test(t) ? 'n ' : ' ') + t);
};
A.instanceOf = A.instanceof = function (ctor) {
  return this._assert(this._obj instanceof ctor, 'expected ' + __inspect(this._obj) + ' to be an instance of ' + (ctor && ctor.name),
    'expected ' + __inspect(this._obj) + ' to not be an instance of ' + (ctor && ctor.name));
};
function __num(name, cmp, word) {
  A[name] = function (n) {
    var o = this._obj;
    var val = this._len ? (o == null ? undefined : o.length) : o;
    var subject = this._len ? 'length of ' + __inspect(o) + ' (' + val + ')' : __inspect(o);
    return this._assert(cmp(val, n), 'expected ' + subject + ' to be ' + word + ' ' + n, 'expected ' + subject + ' to not be ' + word + ' ' + n);
  };
}
__num('above', function (a, b) { return a > b; }, 'above');
__num('gt', function (a, b) { return a > b; }, 'above');
__num('greaterThan', function (a, b) { return a > b; }, 'above');
__num('below', function (a, b) { return a < b; }, 'below');
__num('lt', function (a, b) { return a < b; }, 'below');
__num('lessThan', function (a, b) { return a < b; }, 'below');
__num('least', function (a, b) { return a >= b; }, 'at least');
__num('gte', function (a, b) { return a >= b; }, 'at least');
__num('most', function (a, b) { return a <= b; }, 'at most');
__num('lte', function (a, b) { return a <= b; }, 'at most');
A.within = function (lo, hi) {
  var o = this._len ? this._obj.length : this._obj;
  return this._assert(o >= lo && o <= hi, 'expected ' + __inspect(o) + ' to be within ' + lo + '..' + hi,
    'expected ' + __inspect(o) + ' to not be within ' + lo + '..' + hi);
};
A.closeTo = A.approximately = function (n, delta) {
  return this._assert(Math.abs(this._obj - n) <= delta, 'expected ' + this._obj + ' to be close to ' + n + ' +/- ' + delta,
    'expected ' + this._obj + ' not to be close to ' + n + ' +/- ' + delta);
};
function __lengthImpl(n) {
  var len = this._obj == null ? undefined : this._obj.length;
  return this._assert(len === n, 'expected ' + __inspect(this._obj) + ' to have a length of ' + n + ' but got ' + len,
    'expected ' + __inspect(this._obj) + ' to not have a length of ' + n);
};
A.property = function (name, value) {
  var o = this._obj;
  var found = this._nested ? __getPath(o, name)
    : (o !== null && o !== undefined && (this._own ? Object.prototype.hasOwnProperty.call(o, name) : name in Object(o)))
      ? { exists: true, value: o[name] } : { exists: false };
  if (arguments.length > 1) {
    var match = found.exists && (this._deep ? __deepEqual(found.value, value) : found.value === value);
    this._assert(match, 'expected ' + __inspect(o) + ' to have property ' + __inspect(name) + ' of ' + __inspect(value) + (found.exists ? ', but got ' + __inspect(found.value) : ''),
      'expected ' + __inspect(o) + ' to not have property ' + __inspect(name) + ' of ' + __inspect(value), value, found.value);
  } else {
    this._assert(found.exists, 'expected ' + __inspect(o) + ' to have property ' + __inspect(name),
      'expected ' + __inspect(o) + ' to not have property ' + __inspect(name));
  }
  if (!this._negate) this._obj = found.value;
  return this;
};
A.ownProperty = A.haveOwnProperty = function (name, value) {
  this._own = true;
  return arguments.length > 1 ? this.property(name, value) : this.property(name);
};
A.keys = A.key = function () {
  var want = Array.isArray(arguments[0]) ? arguments[0] : Array.prototype.slice.call(arguments);
  var have = this._obj && typeof this._obj === 'object' ? Object.keys(this._obj) : [];
  var ok = this._any
    ? want.some(function (k) { return have.indexOf(k) >= 0; })
    : want.every(function (k) { return have.indexOf(k) >= 0; }) && (this._contains || have.length === want.length);
  return this._assert(ok, 'expected ' + __inspect(this._obj) + ' to have ' + (this._any ? 'any of ' : '') + 'keys ' + __inspect(want),
    'expected ' + __inspect(this._obj) + ' to not have keys ' + __inspect(want));
};
function __includeImpl(v) {
  var o = this._obj, ok;
  var self = this;
  if (typeof o === 'string') ok = o.indexOf(v) >= 0;
  else if (Array.isArray(o)) ok = o.some(function (x) { return self._deep ? __deepEqual(x, v) : x === v; });
  else if (o && typeof o === 'object' && v && typeof v === 'object') {
    ok = Object.keys(v).every(function (k) { return self._deep ? __deepEqual(o[k], v[k]) : o[k] === v[k]; });
  } else if (o && typeof o === 'object') ok = Object.prototype.hasOwnProperty.call(o, v);
  else ok = false;
  return this._assert(ok, 'expected ' + __inspect(o) + ' to include ' + __inspect(v), 'expected ' + __inspect(o) + ' to not include ' + __inspect(v));
};

// Chainable methods: callable (`include(x)`) and usable as a chain
// (`include.keys(...)`, `length.above(2)`, `a.property(...)`).
function __chainable(names, method, setup) {
  names.forEach(function (name) {
    Object.defineProperty(A, name, {
      configurable: true,
      get: function () {
        var self = this;
        var f = function () { return method.apply(self, arguments); };
        return new Proxy(f, {
          get: function (target, prop) {
            if (prop === 'call' || prop === 'apply' || prop === 'bind') return target[prop];
            if (setup) setup.call(self);
            var v = self[prop];
            return typeof v === 'function' ? v.bind(self) : v;
          },
        });
      },
    });
  });
}
__chainable(['include', 'includes', 'contain', 'contains'], __includeImpl, function () { this._contains = true; });
__chainable(['length', 'lengthOf'], __lengthImpl, function () { this._len = true; });
__chainable(['a', 'an'], __typeImpl, null);
A.match = A.matches = function (re) {
  return this._assert(re.test(this._obj), 'expected ' + __inspect(this._obj) + ' to match ' + re, 'expected ' + __inspect(this._obj) + ' not to match ' + re);
};
A.string = function (sub) {
  return this._assert(typeof this._obj === 'string' && this._obj.indexOf(sub) >= 0,
    'expected ' + __inspect(this._obj) + ' to contain ' + __inspect(sub), 'expected ' + __inspect(this._obj) + ' to not contain ' + __inspect(sub));
};
A.oneOf = function (list) {
  var o = this._obj, deep = this._deep;
  var ok = list.some(function (x) { return deep ? __deepEqual(x, o) : x === o; });
  return this._assert(ok, 'expected ' + __inspect(o) + ' to be one of ' + __inspect(list), 'expected ' + __inspect(o) + ' to not be one of ' + __inspect(list));
};
A.members = function (list) {
  var o = this._obj || [];
  var has = function (arr, x) { return arr.some(function (y) { return __deepEqual(x, y); }); };
  var ok = o.length === list.length && list.every(function (x) { return has(o, x); }) && o.every(function (x) { return has(list, x); });
  return this._assert(ok, 'expected ' + __inspect(o) + ' to have the same members as ' + __inspect(list),
    'expected ' + __inspect(o) + ' to not have the same members as ' + __inspect(list));
};
A.throw = A.throws = A.Throw = function (expected) {
  var threw = false, err;
  try { this._obj(); } catch (e) { threw = true; err = e; }
  var ok = threw;
  if (threw && expected !== undefined) {
    if (typeof expected === 'string') ok = String(err && err.message).indexOf(expected) >= 0;
    else if (expected instanceof RegExp) ok = expected.test(String(err && err.message));
    else if (typeof expected === 'function') ok = err instanceof expected;
  }
  return this._assert(ok, 'expected function to throw' + (expected !== undefined ? ' ' + __inspect(expected) : '') + (threw ? ' but got ' + __inspect(err) : ''),
    'expected function to not throw but it threw ' + __inspect(err));
};
A.satisfy = A.satisfies = function (fn) {
  return this._assert(!!fn(this._obj), 'expected ' + __inspect(this._obj) + ' to satisfy ' + __inspect(fn),
    'expected ' + __inspect(this._obj) + ' to not satisfy ' + __inspect(fn));
};

// Minimal JSON Schema (draft-07 subset) validation for jsonSchema().
function __validateSchema(schema, value, path, errors) {
  path = path || '$';
  if (!schema || typeof schema !== 'object') return errors;
  var typeOf = function (v) {
    if (v === null) return 'null';
    if (Array.isArray(v)) return 'array';
    if (typeof v === 'number' && Math.floor(v) === v) return 'integer';
    return typeof v;
  };
  if (schema.type) {
    var types = Array.isArray(schema.type) ? schema.type : [schema.type];
    var t = typeOf(value);
    var ok = types.some(function (x) { return x === t || (x === 'number' && t === 'integer'); });
    if (!ok) { errors.push(path + ' should be ' + types.join('|') + ' but is ' + t); return errors; }
  }
  if (schema.enum && !schema.enum.some(function (e) { return __deepEqual(e, value); })) errors.push(path + ' should be one of ' + __inspect(schema.enum));
  if (schema.const !== undefined && !__deepEqual(schema.const, value)) errors.push(path + ' should equal ' + __inspect(schema.const));
  if (typeof value === 'number') {
    if (schema.minimum !== undefined && value < schema.minimum) errors.push(path + ' should be >= ' + schema.minimum);
    if (schema.maximum !== undefined && value > schema.maximum) errors.push(path + ' should be <= ' + schema.maximum);
  }
  if (typeof value === 'string') {
    if (schema.minLength !== undefined && value.length < schema.minLength) errors.push(path + ' should be at least ' + schema.minLength + ' characters');
    if (schema.maxLength !== undefined && value.length > schema.maxLength) errors.push(path + ' should be at most ' + schema.maxLength + ' characters');
    if (schema.pattern && !new RegExp(schema.pattern).test(value)) errors.push(path + ' should match ' + schema.pattern);
  }
  if (Array.isArray(value)) {
    if (schema.minItems !== undefined && value.length < schema.minItems) errors.push(path + ' should have at least ' + schema.minItems + ' items');
    if (schema.maxItems !== undefined && value.length > schema.maxItems) errors.push(path + ' should have at most ' + schema.maxItems + ' items');
    if (schema.items) value.forEach(function (v, i) { __validateSchema(schema.items, v, path + '[' + i + ']', errors); });
  }
  if (value && typeof value === 'object' && !Array.isArray(value)) {
    (schema.required || []).forEach(function (k) {
      if (!(k in value)) errors.push(path + ' should have required property ' + __inspect(k));
    });
    var props = schema.properties || {};
    Object.keys(props).forEach(function (k) {
      if (k in value) __validateSchema(props[k], value[k], path + '.' + k, errors);
    });
    if (schema.additionalProperties === false) {
      Object.keys(value).forEach(function (k) { if (!(k in props)) errors.push(path + ' should not have additional property ' + __inspect(k)); });
    }
  }
  ['allOf', 'anyOf', 'oneOf'].forEach(function (kw) {
    if (!schema[kw]) return;
    var results = schema[kw].map(function (s) { return __validateSchema(s, value, path, []).length === 0; });
    var passed = results.filter(Boolean).length;
    if (kw === 'allOf' && passed !== results.length) errors.push(path + ' should match all schemas in allOf');
    if (kw === 'anyOf' && passed === 0) errors.push(path + ' should match some schema in anyOf');
    if (kw === 'oneOf' && passed !== 1) errors.push(path + ' should match exactly one schema in oneOf');
  });
  return errors;
}
A.jsonSchema = function (schema) {
  var subject = this._obj && this._obj.__isResponse ? this._obj.json() : this._obj;
  var errors = __validateSchema(schema, subject, '$', []);
  return this._assert(errors.length === 0, 'expected data to satisfy schema but found errors: ' + errors.join('; '),
    'expected data to not satisfy schema');
};

// ------------------------------------------------------------------ property lists

function PropertyList(items, opts) {
  this._items = items;
  this._ci = !!(opts && opts.caseInsensitive);
  this._onChange = (opts && opts.onChange) || function () {};
  this._readOnly = !!(opts && opts.readOnly);
}
PropertyList.prototype._match = function (item, name) {
  return this._ci ? String(item.key).toLowerCase() === String(name).toLowerCase() : item.key === name;
};
PropertyList.prototype._enabled = function () {
  return this._items.filter(function (i) { return i.enabled !== false; });
};
PropertyList.prototype.get = function (name) {
  var self = this;
  var found = this._enabled().filter(function (i) { return self._match(i, name); });
  return found.length ? found[found.length - 1].value : undefined;
};
PropertyList.prototype.has = function (name, value) {
  var self = this;
  return this._enabled().some(function (i) { return self._match(i, name) && (value === undefined || i.value === value); });
};
PropertyList.prototype.one = function (name) {
  var self = this;
  var f = this._enabled().filter(function (i) { return self._match(i, name); });
  return f.length ? { key: f[0].key, value: f[0].value } : undefined;
};
PropertyList.prototype._norm = function (item) {
  if (typeof item === 'string') {
    var i = item.indexOf(':');
    return { key: item.slice(0, i).trim(), value: item.slice(i + 1).trim(), enabled: true };
  }
  return { key: String(item.key), value: __toStr(item.value), enabled: item.disabled ? false : true };
};
PropertyList.prototype._mutate = function () {
  if (this._readOnly) throw new TypeError('This list is read-only');
};
PropertyList.prototype.add = function (item) {
  this._mutate();
  this._items.push(this._norm(item));
  this._onChange();
};
PropertyList.prototype.append = PropertyList.prototype.add;
PropertyList.prototype.upsert = function (item) {
  this._mutate();
  var n = this._norm(item), self = this;
  var existing = this._items.filter(function (i) { return self._match(i, n.key); });
  if (existing.length) {
    existing[0].value = n.value;
    existing[0].enabled = true;
  } else this._items.push(n);
  this._onChange();
};
PropertyList.prototype.remove = function (pred) {
  this._mutate();
  var self = this;
  var test = typeof pred === 'function' ? pred : function (i) { return self._match(i, pred); };
  for (var i = this._items.length - 1; i >= 0; i--) if (test(this._items[i])) this._items.splice(i, 1);
  this._onChange();
};
PropertyList.prototype.clear = function () {
  this._mutate();
  this._items.length = 0;
  this._onChange();
};
PropertyList.prototype.toObject = function () {
  var o = {};
  this._enabled().forEach(function (i) { o[i.key] = i.value; });
  return o;
};
PropertyList.prototype.all = function () {
  return this._enabled().map(function (i) { return { key: i.key, value: i.value }; });
};
PropertyList.prototype.each = function (fn) { this.all().forEach(fn); };
PropertyList.prototype.map = function (fn) { return this.all().map(fn); };
PropertyList.prototype.filter = function (fn) { return this.all().filter(fn); };
PropertyList.prototype.find = function (fn) { return this.all().find(fn); };
PropertyList.prototype.count = function () { return this._enabled().length; };
PropertyList.prototype.idx = function (i) { return this.all()[i]; };
PropertyList.prototype.toString = function () {
  return this.all().map(function (i) { return i.key + ': ' + i.value; }).join('\n');
};

// ------------------------------------------------------------------ variables

function __dyn(name) {
  return __courier_dynamic(name);
}

function __resolveWith(lookup, text, depth) {
  depth = depth || 0;
  return String(text).replace(/\{\{([^{}]+)\}\}/g, function (m, name) {
    var key = name.trim();
    var v = lookup(key);
    if (v === undefined && key.charAt(0) === '$') v = __dyn(key);
    if (v === undefined || depth > 19) return m;
    return __resolveWith(lookup, __toStr(v), depth + 1);
  });
}

function VariableScope(list, name, readOnly) {
  this._list = list;
  this.name = name;
  this._readOnly = readOnly;
}
VariableScope.prototype._find = function (k) {
  for (var i = 0; i < this._list.length; i++) if (this._list[i].key === k) return this._list[i];
  return undefined;
};
VariableScope.prototype.get = function (k) {
  var v = this._find(k);
  if (!v || v.enabled === false) return undefined;
  return v.current !== null && v.current !== undefined ? v.current : v.initial;
};
VariableScope.prototype.has = function (k) {
  var v = this._find(k);
  return !!v && v.enabled !== false;
};
VariableScope.prototype.set = function (k, value) {
  if (this._readOnly) throw new TypeError(this.name + ' variables are read-only');
  var v = this._find(k), s = __toStr(value);
  if (v) {
    v.current = s;
    v.enabled = true;
  } else this._list.push({ key: String(k), initial: '', current: s, enabled: true, secret: false });
};
VariableScope.prototype.unset = function (k) {
  for (var i = this._list.length - 1; i >= 0; i--) if (this._list[i].key === k) this._list.splice(i, 1);
};
VariableScope.prototype.clear = function () { this._list.length = 0; };
VariableScope.prototype.toObject = function () {
  var o = {}, self = this;
  this._list.forEach(function (v) { if (v.enabled !== false) o[v.key] = self.get(v.key); });
  return o;
};
VariableScope.prototype.toJSON = function () { return this.toObject(); };
VariableScope.prototype.replaceIn = function (text) {
  var self = this;
  return __resolveWith(function (k) { return self.get(k); }, text);
};

var __vars = __in.variables;
var __envList = __vars.environment || [];
var __env = new VariableScope(__envList, __vars.environment ? __vars.environmentName : '');
var __coll = new VariableScope(__vars.collection, 'collection');
var __glob = new VariableScope(__vars.globals, 'globals');
var __local = __vars.local; // plain object
var __dataObj = __vars.data; // plain object

function __lookupAll(k) {
  if (Object.prototype.hasOwnProperty.call(__local, k)) return __local[k];
  if (Object.prototype.hasOwnProperty.call(__dataObj, k)) return __dataObj[k];
  if (__vars.environment && __env.has(k)) return __env.get(k);
  if (__coll.has(k)) return __coll.get(k);
  if (__glob.has(k)) return __glob.get(k);
  return undefined;
}

// ------------------------------------------------------------------ request

function __parseQuery(raw) {
  var q = String(raw).split('#')[0];
  var i = q.indexOf('?');
  if (i < 0) return [];
  return q.slice(i + 1).split('&').filter(function (p) { return p !== ''; }).map(function (p) {
    var j = p.indexOf('=');
    return j < 0 ? { key: p, value: '', enabled: true } : { key: p.slice(0, j), value: p.slice(j + 1), enabled: true };
  });
}

function __makeRequest(req) {
  var api = {};
  function syncUrlFromQuery() {
    var raw = req.url, hash = '', hi = raw.indexOf('#');
    if (hi >= 0) { hash = raw.slice(hi); raw = raw.slice(0, hi); }
    var base = raw.split('?')[0];
    var parts = req.query.filter(function (q) { return q.enabled !== false; }).map(function (q) {
      return q.value === '' ? q.key : q.key + '=' + q.value;
    });
    req.url = base + (parts.length ? '?' + parts.join('&') : '') + hash;
  }
  var query = new PropertyList(req.query, { onChange: syncUrlFromQuery });
  var urlObj = {
    toString: function () { return req.url; },
    getHost: function () {
      var m = /^[a-z][a-z0-9+.-]*:\/\/([^/?#:]+)/i.exec(req.url) || /^([^/?#:]+)/.exec(req.url);
      return m ? m[1] : '';
    },
    getPath: function () {
      var r = req.url.replace(/^[a-z][a-z0-9+.-]*:\/\/[^/?#]*/i, '');
      return r.split(/[?#]/)[0] || '/';
    },
    getQueryString: function () {
      var r = req.url.split('#')[0];
      var i = r.indexOf('?');
      return i < 0 ? '' : r.slice(i + 1);
    },
    update: function (s) {
      req.url = String(s);
      req.query.length = 0;
      __parseQuery(req.url).forEach(function (q) { req.query.push(q); });
    },
    query: query,
  };
  Object.defineProperty(api, 'url', {
    get: function () { return urlObj; },
    set: function (v) { urlObj.update(v); },
    enumerable: true,
  });
  Object.defineProperty(api, 'method', {
    get: function () { return req.method; },
    set: function (v) { req.method = String(v).toUpperCase(); },
    enumerable: true,
  });
  api.headers = new PropertyList(req.headers, { caseInsensitive: true });
  api.addHeader = function (h) { api.headers.add(h); };
  api.removeHeader = function (n) { api.headers.remove(n); };
  api.upsertHeader = function (h) { api.headers.upsert(h); };
  api.body = {
    get mode() { return req.body.mode; },
    set mode(m) { req.body.mode = m; },
    get raw() { return req.body.raw; },
    set raw(v) { req.body.raw = __toStr(v); },
    urlencoded: new PropertyList(req.body.urlencoded),
    formdata: new PropertyList(req.body.formdata),
    graphql: req.body.graphql,
    update: function (v) {
      if (typeof v === 'string') {
        req.body.mode = 'raw';
        req.body.raw = v;
      } else if (v && v.mode) {
        req.body.mode = v.mode;
        if (v.raw !== undefined) req.body.raw = __toStr(v.raw);
        if (v.urlencoded) req.body.urlencoded = v.urlencoded.map(function (x) { return { key: x.key, value: __toStr(x.value), enabled: !x.disabled }; });
      }
    },
    toString: function () {
      if (req.body.mode === 'raw') return req.body.raw;
      if (req.body.mode === 'urlencoded') return req.body.urlencoded.filter(function (x) { return x.enabled !== false; }).map(function (x) { return x.key + '=' + x.value; }).join('&');
      return '';
    },
    isEmpty: function () { return req.body.mode === 'none'; },
  };
  api.auth = req.auth;
  api.id = __in.info.requestId;
  api.name = __in.info.requestName;
  api.toJSON = function () { return req; };
  return api;
}

// ------------------------------------------------------------------ response

function __makeResponse(r) {
  if (!r) return undefined;
  var headers = new PropertyList(r.headers.map(function (h) { return { key: h[0], value: h[1], enabled: true }; }), { caseInsensitive: true, readOnly: true });
  var res = {
    __isResponse: true,
    code: r.code,
    status: r.status,
    headers: headers,
    responseTime: r.responseTime,
    responseSize: r.responseSize,
    text: function () { return r.body; },
    json: function () {
      try {
        return JSON.parse(r.body);
      } catch (e) {
        throw new SyntaxError('Response is not valid JSON: ' + e.message);
      }
    },
    reason: function () { return r.status; },
    size: function () { return { body: r.responseSize, header: 0, total: r.responseSize }; },
    cookies: new PropertyList((r.cookies || []).map(function (c) { return { key: c.name, value: c.value, enabled: true }; }), { readOnly: true }),
  };
  res.stream = r.body;
  function check(ok, msg, neg) {
    return { ok: ok, msg: msg, neg: neg };
  }
  function makeTo(negate) {
    function run(c) {
      var ok = negate ? !c.ok : c.ok;
      if (!ok) throw new AssertionError(negate ? c.neg : c.msg);
      return to;
    }
    var codeIs = function (lo, hi) { return r.code >= lo && r.code <= hi; };
    var to = {
      get not() { return makeTo(!negate); },
      get and() { return to; },
      get to() { return to; },
      get be() { return to; },
      get have() { return to; },
      get a() { return to; },
      get an() { return to; },
      status: function (s) {
        if (typeof s === 'number') return run(check(r.code === s, 'expected response to have status code ' + s + ' but got ' + r.code, 'expected response to not have status code ' + s));
        return run(check(r.status === s, 'expected response to have status reason ' + __inspect(s) + ' but got ' + __inspect(r.status), 'expected response to not have status reason ' + __inspect(s)));
      },
      header: function (name, value) {
        var has = headers.has(name), got = headers.get(name);
        if (value === undefined) return run(check(has, 'expected response to have header ' + name, 'expected response to not have header ' + name));
        return run(check(has && got === value, 'expected response to have header ' + name + ' with value ' + __inspect(value) + ' but got ' + __inspect(got), 'expected response to not have header ' + name + ' with value ' + __inspect(value)));
      },
      body: function (b) {
        if (b === undefined) return run(check(r.body.length > 0, 'expected response to have a body', 'expected response to not have a body'));
        if (b instanceof RegExp) return run(check(b.test(r.body), 'expected response body to match ' + b, 'expected response body not to match ' + b));
        return run(check(r.body === b, 'expected response body to equal ' + __inspect(b), 'expected response body to not equal ' + __inspect(b)));
      },
      jsonBody: function (path, value) {
        var data;
        try { data = JSON.parse(r.body); } catch (e) { return run(check(false, 'expected response body to be valid JSON', '')); }
        if (path === undefined) return run(check(true, '', 'expected response to not have a JSON body'));
        if (typeof path === 'object') return run(check(__deepEqual(data, path), 'expected response JSON to deeply equal ' + __inspect(path), 'expected response JSON to not deeply equal ' + __inspect(path)));
        var f = __getPath(data, path);
        if (arguments.length < 2) return run(check(f.exists, 'expected response JSON to have path ' + __inspect(path), 'expected response JSON to not have path ' + __inspect(path)));
        return run(check(f.exists && __deepEqual(f.value, value), 'expected response JSON at ' + __inspect(path) + ' to equal ' + __inspect(value) + ' but got ' + __inspect(f.value), 'expected response JSON at ' + __inspect(path) + ' to not equal ' + __inspect(value)));
      },
      jsonSchema: function (schema) {
        var errors = __validateSchema(schema, res.json(), '$', []);
        return run(check(errors.length === 0, 'expected response to satisfy schema: ' + errors.join('; '), 'expected response to not satisfy schema'));
      },
      get ok() { return run(check(r.code === 200, 'expected response code to be 200 but got ' + r.code, 'expected response code to not be 200')); },
      get success() { return run(check(codeIs(200, 299), 'expected response code to be 2XX but got ' + r.code, 'expected response code to not be 2XX')); },
      get info() { return run(check(codeIs(100, 199), 'expected response code to be 1XX but got ' + r.code, 'expected response code to not be 1XX')); },
      get redirection() { return run(check(codeIs(300, 399), 'expected response code to be 3XX but got ' + r.code, 'expected response code to not be 3XX')); },
      get clientError() { return run(check(codeIs(400, 499), 'expected response code to be 4XX but got ' + r.code, 'expected response code to not be 4XX')); },
      get serverError() { return run(check(codeIs(500, 599), 'expected response code to be 5XX but got ' + r.code, 'expected response code to not be 5XX')); },
      get error() { return run(check(codeIs(400, 599), 'expected response code to be 4XX or 5XX but got ' + r.code, 'expected response code to not be 4XX or 5XX')); },
      get notFound() { return run(check(r.code === 404, 'expected response code to be 404 but got ' + r.code, 'expected response code to not be 404')); },
      get unauthorized() { return run(check(r.code === 401, 'expected response code to be 401 but got ' + r.code, 'expected response code to not be 401')); },
      get forbidden() { return run(check(r.code === 403, 'expected response code to be 403 but got ' + r.code, 'expected response code to not be 403')); },
      get badRequest() { return run(check(r.code === 400, 'expected response code to be 400 but got ' + r.code, 'expected response code to not be 400')); },
      get json() {
        var ok = true;
        try { JSON.parse(r.body); } catch (e) { ok = false; }
        return run(check(ok, 'expected response body to be valid JSON', 'expected response body to not be valid JSON'));
      },
      get withBody() { return to.body(); },
      get responseTime() {
        return {
          below: function (ms) { return run(check(r.responseTime < ms, 'expected response time ' + r.responseTime + 'ms to be below ' + ms + 'ms', 'expected response time to not be below ' + ms + 'ms')); },
          above: function (ms) { return run(check(r.responseTime > ms, 'expected response time ' + r.responseTime + 'ms to be above ' + ms + 'ms', 'expected response time to not be above ' + ms + 'ms')); },
        };
      },
    };
    return to;
  }
  Object.defineProperty(res, 'to', { get: function () { return makeTo(false); } });
  return res;
}

// ------------------------------------------------------------------ tests

function __record(result) {
  __state.tests.push(result);
  return result;
}

function __errMessage(e) {
  if (e && e.message !== undefined) return (e.name && e.name !== 'Error' && e.name !== 'AssertionError' ? e.name + ': ' : '') + e.message;
  return __inspect(e);
}

function __test(name, fn) {
  var result = __record({ name: String(name), passed: true, skipped: false, error: null, event: __in.event });
  if (typeof fn !== 'function') return pm;
  var fail = function (e) { result.passed = false; result.error = __errMessage(e); };
  try {
    if (fn.length > 0) {
      result.pending = true;
      __state.pending++;
      var settled = false;
      var done = function (err) {
        if (settled) return;
        settled = true;
        __state.pending--;
        delete result.pending;
        if (err) fail(err);
      };
      fn(done);
    } else {
      var ret = fn();
      if (ret && typeof ret.then === 'function') {
        result.pending = true;
        __state.pending++;
        ret.then(
          function () { __state.pending--; delete result.pending; },
          function (e) { __state.pending--; delete result.pending; fail(e); }
        );
      }
    }
  } catch (e) {
    fail(e);
  }
  return pm;
}
__test.skip = function (name) {
  __record({ name: String(name), passed: false, skipped: true, error: null, event: __in.event });
  return pm;
};

// ------------------------------------------------------------------ pm

function __normalizeSendRequest(req) {
  if (typeof req === 'string') return { url: req, method: 'GET', header: [], body: null };
  var headers = [];
  var h = req.header || req.headers;
  if (Array.isArray(h)) headers = h.map(function (x) { return typeof x === 'string' ? PropertyList.prototype._norm(x) : { key: x.key, value: __toStr(x.value) }; });
  else if (h && typeof h === 'object') headers = Object.keys(h).map(function (k) { return { key: k, value: __toStr(h[k]) }; });
  var body = req.body || null;
  if (body && body.urlencoded) body = { mode: 'urlencoded', urlencoded: body.urlencoded.map(function (x) { return { key: x.key, value: __toStr(x.value) }; }) };
  else if (body && body.mode === 'raw') body = { mode: 'raw', raw: __toStr(body.raw) };
  else if (body && body.raw !== undefined) body = { mode: 'raw', raw: __toStr(body.raw) };
  return { url: __toStr(req.url), method: String(req.method || 'GET').toUpperCase(), header: headers, body: body };
}

var pm = {
  info: __in.info,
  variables: {
    get: function (k) { return __lookupAll(k); },
    has: function (k) { return __lookupAll(k) !== undefined; },
    set: function (k, v) { __local[k] = __toStr(v); },
    unset: function (k) { delete __local[k]; },
    replaceIn: function (t) { return __resolveWith(__lookupAll, t); },
    toObject: function () {
      var o = {};
      [__glob.toObject(), __coll.toObject(), __vars.environment ? __env.toObject() : {}, __dataObj, __local].forEach(function (s) {
        Object.keys(s).forEach(function (k) { o[k] = s[k]; });
      });
      return o;
    },
  },
  environment: __env,
  collectionVariables: __coll,
  globals: __glob,
  iterationData: {
    get: function (k) { return __dataObj[k]; },
    has: function (k) { return Object.prototype.hasOwnProperty.call(__dataObj, k); },
    toObject: function () { return JSON.parse(JSON.stringify(__dataObj)); },
    toJSON: function () { return __dataObj; },
  },
  request: __makeRequest(__in.request),
  response: __makeResponse(__in.response),
  test: __test,
  expect: function (v, msg) { return new Assertion(v, msg); },
  execution: {
    setNextRequest: function (n) {
      __state.nextSet = true;
      __state.next = n === null || n === undefined ? null : String(n);
    },
    skipRequest: function () {
      if (__in.event !== 'prerequest') throw new Error('skipRequest() can only be used in pre-request scripts');
      __state.skip = true;
    },
    location: [__in.info.requestName],
  },
  sendRequest: function (req, cb) {
    var out = JSON.parse(__courier_send(JSON.stringify(__normalizeSendRequest(req))));
    var err = out.error ? new Error(out.error) : null;
    var res = out.error ? undefined : __makeResponse(out.response);
    if (typeof cb === 'function') {
      try { cb(err, res); } catch (e) { __state.console.push({ level: 'error', message: 'Error in sendRequest callback: ' + __errMessage(e) }); throw e; }
      return;
    }
    return err ? Promise.reject(err) : Promise.resolve(res);
  },
  cookies: new PropertyList((__in.cookies || []).map(function (c) { return { key: c.name, value: c.value, enabled: true }; }), { readOnly: true }),
  visualizer: {
    set: function (template, data) {
      __state.visualizer = { template: String(template), data: data === undefined ? null : data };
      console.info('pm.visualizer is not rendered in this version of Courier.');
    },
    clear: function () { __state.visualizer = null; },
  },
  require: function (name) { return require(name); },
};
pm.execution.setNextRequest.toString = function () { return 'setNextRequest'; };
pm.cookies.jar = function () {
  function call(op, url, name, value, cb) {
    var out = JSON.parse(__courier_jar(JSON.stringify({ op: op, url: __toStr(url), name: name, value: value })));
    if (typeof cb === 'function') cb(out.error ? new Error(out.error) : null, out.result);
    return out.result;
  }
  return {
    get: function (url, name, cb) { return call('get', url, name, null, cb); },
    getAll: function (url, cb) { return call('getAll', url, null, null, cb); },
    set: function (url, name, value, cb) {
      if (name && typeof name === 'object') { cb = value; value = name.value; name = name.name || name.key; }
      return call('set', url, String(name), __toStr(value), cb);
    },
    unset: function (url, name, cb) { return call('unset', url, name, null, cb); },
    clear: function (url, cb) { return call('clear', url, null, null, cb); },
  };
};

function require(name) {
  switch (name) {
    case 'uuid':
      return { v4: function () { return __courier_uuid(); } };
    case 'crypto-js':
      return CryptoJS;
    case 'chai':
      return { expect: pm.expect, AssertionError: AssertionError };
    case 'xml2js':
      return { parseString: function (x, cb) { try { cb(null, xml2Json(x)); } catch (e) { cb(e); } } };
    default:
      throw new Error("Module '" + name + "' is not available in Courier scripts");
  }
}

// ------------------------------------------------------------------ legacy globals

var tests = {};
var postman = {
  setNextRequest: pm.execution.setNextRequest,
  setEnvironmentVariable: function (k, v) { __env.set(k, v); },
  getEnvironmentVariable: function (k) { return __env.get(k); },
  clearEnvironmentVariable: function (k) { __env.unset(k); },
  clearEnvironmentVariables: function () { __env.clear(); },
  setGlobalVariable: function (k, v) { __glob.set(k, v); },
  getGlobalVariable: function (k) { return __glob.get(k); },
  clearGlobalVariable: function (k) { __glob.unset(k); },
  clearGlobalVariables: function () { __glob.clear(); },
  getResponseHeader: function (n) { return pm.response ? pm.response.headers.get(n) : undefined; },
  getResponseCookie: function (n) { return pm.cookies.has(n) ? { name: n, value: pm.cookies.get(n) } : undefined; },
};
var environment = __env.toObject();
var globals = __glob.toObject();
var data = __dataObj;
var iteration = __in.info.iteration;
var request = {
  url: __in.request.url,
  method: __in.request.method,
  headers: pm.request.headers.toObject(),
  data: pm.request.body.toString(),
  name: __in.info.requestName,
};
var responseBody, responseCode, responseTime, responseHeaders, responseCookies;
if (pm.response) {
  responseBody = pm.response.text();
  responseCode = { code: pm.response.code, name: pm.response.status, detail: pm.response.status };
  responseTime = pm.response.responseTime;
  responseHeaders = pm.response.headers.toObject();
  responseCookies = pm.cookies.all().map(function (c) { return { name: c.key, value: c.value }; });
}

// ------------------------------------------------------------------ host hooks

function __courier_fail(e) {
  var msg = e && e.message !== undefined ? (e.name || 'Error') + ': ' + e.message : 'Uncaught ' + __inspect(e);
  var m = e && e.stack && /eval_script:(\d+)/.exec(e.stack);
  if (m) msg += ' (line ' + m[1] + ')';
  __state.error = msg;
}

function __courier_take_error() {
  var e = __state.error || '';
  __state.error = null;
  return e;
}

function __courier_syntax_check(src) {
  try {
    new Function(src);
    return '';
  } catch (e) {
    return (e.name || 'SyntaxError') + ': ' + e.message;
  }
}

function __courier_collect_legacy() {
  Object.keys(tests).forEach(function (name) {
    __record({ name: name, passed: !!tests[name], skipped: false, error: tests[name] ? null : 'test returned false', event: __in.event });
  });
  tests = {};
}

function __courier_output() {
  __state.tests.forEach(function (t) {
    if (t.pending) {
      t.passed = false;
      t.error = 'test did not complete (done() was never called)';
      delete t.pending;
    }
  });
  return JSON.stringify({
    tests: __state.tests,
    console: __state.console,
    nextSet: __state.nextSet,
    next: __state.next,
    skip: __state.skip,
    visualizer: __state.visualizer,
    request: __in.request,
    variables: {
      local: __local,
      environment: __vars.environment ? __envList : null,
      collection: __vars.collection,
      globals: __vars.globals,
    },
  });
}
