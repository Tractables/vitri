// A store-only zip writer, for saving the bundle as one file.

const CRC_TABLE = (() => {
  const table = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    table[n] = c >>> 0;
  }
  return table;
})();

export function crc32(bytes) {
  let c = 0xffffffff;
  for (let i = 0; i < bytes.length; i++) c = CRC_TABLE[(c ^ bytes[i]) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

// A zip archive of the given files, stored without compression: a local
// header and the bytes per file, then the central directory and its end
// record. Names are UTF-8 (flag bit 11). Zip64 is not written, so the whole
// archive stays under 4 GiB and 65,535 files.
export function zipStore(entries, date = new Date()) {
  const encoder = new TextEncoder();
  const year = Math.max(1980, date.getFullYear());
  const time = (date.getHours() << 11) | (date.getMinutes() << 5) | (date.getSeconds() >> 1);
  const day = ((year - 1980) << 9) | ((date.getMonth() + 1) << 5) | date.getDate();
  const files = entries.map(({ name, data }) => {
    const bytes = typeof data === "string" ? encoder.encode(data) : data;
    return { name: encoder.encode(name), bytes, crc: crc32(bytes) };
  });
  if (files.length > 0xffff) throw new Error("more files than a zip without Zip64 holds");
  let size = 22;
  for (const file of files) size += 30 + file.name.length + file.bytes.length + 46 + file.name.length;
  if (size > 0xffffffff) throw new Error("the bundle is larger than a zip without Zip64 holds");

  const out = new Uint8Array(size);
  const view = new DataView(out.buffer);
  let at = 0;
  const u16 = (value) => {
    view.setUint16(at, value, true);
    at += 2;
  };
  const u32 = (value) => {
    view.setUint32(at, value, true);
    at += 4;
  };
  const offsets = [];
  for (const file of files) {
    offsets.push(at);
    u32(0x04034b50);
    u16(20);
    u16(0x0800);
    u16(0);
    u16(time);
    u16(day);
    u32(file.crc);
    u32(file.bytes.length);
    u32(file.bytes.length);
    u16(file.name.length);
    u16(0);
    out.set(file.name, at);
    at += file.name.length;
    out.set(file.bytes, at);
    at += file.bytes.length;
  }
  const directory = at;
  files.forEach((file, i) => {
    u32(0x02014b50);
    u16(20);
    u16(20);
    u16(0x0800);
    u16(0);
    u16(time);
    u16(day);
    u32(file.crc);
    u32(file.bytes.length);
    u32(file.bytes.length);
    u16(file.name.length);
    u16(0);
    u16(0);
    u16(0);
    u16(0);
    u32(0);
    u32(offsets[i]);
    out.set(file.name, at);
    at += file.name.length;
  });
  const directorySize = at - directory;
  u32(0x06054b50);
  u16(0);
  u16(0);
  u16(files.length);
  u16(files.length);
  u32(directorySize);
  u32(directory);
  u16(0);
  return out;
}
