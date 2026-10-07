/**
 * StateFS Universal C-ABI Header
 *
 * High-performance hierarchical in-memory state and virtual configuration engine.
 */

#ifndef STATEFS_H
#define STATEFS_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct StatefsStore StatefsStore;

/**
 * Retrieves the most recent error message on the calling thread.
 * Returns bytes copied into buf. If no error occurred, returns 0 and writes empty string.
 */
size_t statefs_last_error(char* buf, size_t max_len);

/**
 * Creates and initializes a new empty StatefsStore instance.
 * Must be deallocated with statefs_store_free.
 */
StatefsStore* statefs_store_new(void);

/**
 * Deallocates a previously created StatefsStore. Safe no-op if store is NULL.
 */
void statefs_store_free(StatefsStore* store);

/**
 * Inserts a string value at path. Returns 0 on success, -1 on error.
 */
int32_t statefs_store_insert_str(StatefsStore* store, const char* path, const char* val);

/**
 * Inserts a 64-bit signed integer value at path. Returns 0 on success, -1 on error.
 */
int32_t statefs_store_insert_int(StatefsStore* store, const char* path, int64_t val);

/**
 * Inserts a boolean value (0 = false, non-zero = true). Returns 0 on success, -1 on error.
 */
int32_t statefs_store_insert_bool(StatefsStore* store, const char* path, int32_t val);

/**
 * Inserts a 64-bit float value at path. Returns 0 on success, -1 on error.
 */
int32_t statefs_store_insert_float(StatefsStore* store, const char* path, double val);

/**
 * Reads a string value from path into buf.
 * Returns 0 on success, 1 if path not found, -1 on error or type mismatch.
 */
int32_t statefs_store_get_str(const StatefsStore* store, const char* path, char* buf, size_t buf_len, size_t* written);

/**
 * Reads a 64-bit signed integer from path into val_out.
 * Returns 0 on success, 1 if path not found, -1 on error or type mismatch.
 */
int32_t statefs_store_get_int(const StatefsStore* store, const char* path, int64_t* val_out);

/**
 * Reads a boolean from path into val_out (0 = false, 1 = true).
 * Returns 0 on success, 1 if path not found, -1 on error or type mismatch.
 */
int32_t statefs_store_get_bool(const StatefsStore* store, const char* path, int32_t* val_out);

/**
 * Reads a float from path into val_out.
 * Returns 0 on success, 1 if path not found, -1 on error or type mismatch.
 */
int32_t statefs_store_get_float(const StatefsStore* store, const char* path, double* val_out);

/**
 * Checks if a node exists at path. Returns 1 if present, 0 if absent, -1 on error.
 */
int32_t statefs_store_contains(const StatefsStore* store, const char* path);

/**
 * Removes the node at path. Returns 0 on success, 1 if not found, -1 on error.
 */
int32_t statefs_store_remove(StatefsStore* store, const char* path);

/**
 * Ingests a JSON string into store. Returns 0 on success, -1 on error.
 */
int32_t statefs_store_load_json_str(StatefsStore* store, const char* json_str);

/**
 * Ingests a TOML string into store. Returns 0 on success, -1 on error.
 */
int32_t statefs_store_load_toml_str(StatefsStore* store, const char* toml_str);

/**
 * Ingests system environment variables matching prefix. Returns count of ingested items, -1 on error.
 */
int32_t statefs_store_load_env(StatefsStore* store, const char* prefix, const char* separator);

/**
 * Exports current store into a binary snapshot.
 * The buffer out_buf must be freed with statefs_free_bytes.
 */
int32_t statefs_store_export_snapshot(const StatefsStore* store, uint8_t** out_buf, size_t* out_len);

/**
 * Restores a StatefsStore from a binary snapshot.
 * Returns pointer to new store or NULL on error.
 */
StatefsStore* statefs_store_restore_snapshot(const uint8_t* data, size_t len);

/**
 * Frees bytes allocated by statefs_store_export_snapshot. Safe no-op on NULL.
 */
void statefs_free_bytes(uint8_t* ptr, size_t len);

#ifdef __cplusplus
}
#endif

#endif /* STATEFS_H */
