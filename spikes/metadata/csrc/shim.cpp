// Phase 0 spike 03: everything the import fingerprint needs from one LibRaw open.
#include <libraw/libraw.h>
#include <cstring>
#include <memory>

extern "C" {

typedef struct {
  char make[64], model[64], serial[64];
  char dto[24], subsec[8], offset[8], exif_serial[64];
  long long timestamp;
  unsigned shot_order;
  int preview_count;
} mm_meta;

static void copy_str(LibRaw_abstract_datastream *s, int len, char *dst, int cap) {
  if (dst[0]) return;  // first occurrence wins (EXIF IFD comes before any thumbnails' IFDs)
  int n = len < cap - 1 ? len : cap - 1;
  s->read(dst, 1, n);
  dst[n] = 0;
}

// LibRaw calls this for every tag it walks. The EXIF IFD arrives with no context prefix
// (IFDn tags arrive as tag | (n+1) << 20); LibRaw restores the stream position afterwards.
static void exif_cb(void *ctx, int tag, int type, int len, unsigned, void *ifp, INT64) {
  mm_meta *m = (mm_meta *)ctx;
  auto *s = (LibRaw_abstract_datastream *)ifp;
  if (type != 2) return;
  switch (tag) {
    case 0x9003: copy_str(s, len, m->dto, sizeof m->dto); break;            // DateTimeOriginal
    case 0x9291: copy_str(s, len, m->subsec, sizeof m->subsec); break;      // SubSecTimeOriginal
    case 0x9011: copy_str(s, len, m->offset, sizeof m->offset); break;      // OffsetTimeOriginal
    case 0xa431: copy_str(s, len, m->exif_serial, sizeof m->exif_serial); break;  // BodySerialNumber
  }
}

int mm_meta_read(const char *path, mm_meta *m) {
  memset(m, 0, sizeof *m);
  // LibRaw is several hundred KB; keep it off worker-thread stacks.
  auto owner = std::make_unique<LibRaw>();
  LibRaw &lr = *owner;
  lr.set_exifparser_handler(exif_cb, m);
  int rc = lr.open_file(path);
  if (rc != LIBRAW_SUCCESS) return rc;
  strncpy(m->make, lr.imgdata.idata.make, 63);
  strncpy(m->model, lr.imgdata.idata.model, 63);
  strncpy(m->serial, lr.imgdata.shootinginfo.BodySerial, 63);
  m->timestamp = (long long)lr.imgdata.other.timestamp;
  m->shot_order = lr.imgdata.other.shot_order;
  m->preview_count = lr.imgdata.thumbs_list.thumbcount;
  return 0;
}

}
