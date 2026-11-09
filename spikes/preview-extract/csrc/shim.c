// Minimal C shim over LibRaw so the spike doesn't depend on Rust struct layouts of libraw_data_t.
#include <libraw/libraw.h>
#include <string.h>

typedef struct {
  const unsigned char *data;
  size_t len;
  int width, height, is_jpeg, thumb_count;
  char make[64], model[64], lens[128], serial[64];
  long long timestamp;
  float iso;
  void *_img, *_lr;
} mm_thumb;

typedef struct {
  int width, height, format;
  unsigned long long offset, len;
} mm_thumb_item;

// Lists every embedded preview with its location in the file, without decoding anything.
// Returns the number of items written, or a negative LibRaw error code.
int mm_libraw_list(const char *path, mm_thumb_item *items, int max) {
  libraw_data_t *lr = libraw_init(0);
  if (!lr) return -1;
  int rc = libraw_open_file(lr, path);
  if (rc != LIBRAW_SUCCESS) { libraw_close(lr); return rc; }
  int n = lr->thumbs_list.thumbcount < max ? lr->thumbs_list.thumbcount : max;
  for (int i = 0; i < n; i++) {
    libraw_thumbnail_item_t *t = &lr->thumbs_list.thumblist[i];
    items[i].width = t->twidth;
    items[i].height = t->theight;
    items[i].format = t->tformat;
    items[i].offset = (unsigned long long)t->toffset;
    items[i].len = t->tlength;
  }
  libraw_close(lr);
  return n;
}

int mm_libraw_thumb(const char *path, mm_thumb *out) {
  memset(out, 0, sizeof *out);
  libraw_data_t *lr = libraw_init(0);
  if (!lr) return -1;
  out->_lr = lr;
  int rc = libraw_open_file(lr, path);
  if (rc != LIBRAW_SUCCESS) return rc;
  strncpy(out->make, lr->idata.make, 63);
  strncpy(out->model, lr->idata.model, 63);
  strncpy(out->lens, lr->lens.Lens, 127);
  strncpy(out->serial, lr->shootinginfo.BodySerial, 63);
  out->timestamp = (long long)lr->other.timestamp;
  out->iso = lr->other.iso_speed;
  out->thumb_count = lr->thumbs_list.thumbcount;
  rc = libraw_unpack_thumb(lr);
  if (rc != LIBRAW_SUCCESS) return rc;
  out->width = lr->thumbnail.twidth;
  out->height = lr->thumbnail.theight;
  libraw_processed_image_t *img = libraw_dcraw_make_mem_thumb(lr, &rc);
  if (!img) return rc;
  out->_img = img;
  out->data = img->data;
  out->len = img->data_size;
  out->is_jpeg = img->type == LIBRAW_IMAGE_JPEG;
  return 0;
}

void mm_libraw_free(mm_thumb *t) {
  if (t->_img) libraw_dcraw_clear_mem((libraw_processed_image_t *)t->_img);
  if (t->_lr) libraw_close((libraw_data_t *)t->_lr);
  t->_img = t->_lr = 0;
}
