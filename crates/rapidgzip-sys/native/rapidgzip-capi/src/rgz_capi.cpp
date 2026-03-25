#include <cstdint>
#include <cstring>
#include <memory>
#include <string>
#include <vector>
#include <iostream>
#include <algorithm>
#include <fstream>
#include <limits>

// Include rapidgzip headers
#include <rapidgzip/ParallelGzipReader.hpp>
#include <filereader/Standard.hpp>
#include <filereader/FileReader.hpp>
#include <filereader/Shared.hpp>

extern "C" {

#define RGZ_ABI_VERSION_MAJOR 0
#define RGZ_ABI_VERSION_MINOR 1

typedef enum {
    RGZ_STATUS_OK = 0,
    RGZ_STATUS_EOF = 1,
    RGZ_STATUS_INVALID_ARGUMENT = 10,
    RGZ_STATUS_IO_ERROR = 11,
    RGZ_STATUS_DATA_ERROR = 12,
    RGZ_STATUS_UNSUPPORTED = 13,
    RGZ_STATUS_OUT_OF_MEMORY = 14,
    RGZ_STATUS_INTERNAL_ERROR = 15,
    RGZ_STATUS_SEEK_ERROR = 16,
    RGZ_STATUS_INDEX_ERROR = 17,
    RGZ_STATUS_STATE_ERROR = 18,
    RGZ_STATUS_NOT_IMPLEMENTED = 19
} rgz_status_t;

typedef struct {
    uint32_t struct_size;
    uint32_t flags;
    uint32_t parallelism;
    uint32_t reserved0;
    uint64_t chunk_size;
    uint64_t reserved1;
} rgz_config_t;

static constexpr uint32_t RGZ_IO_READ_MODE_MASK = 0b11U;
static constexpr uint32_t RGZ_IO_READ_MODE_AUTO = 0U;
static constexpr uint32_t RGZ_IO_READ_MODE_SEQUENTIAL = 1U;
static constexpr uint32_t RGZ_IO_READ_MODE_PREAD = 2U;
static constexpr uint32_t RGZ_IO_READ_MODE_LOCKED_READ_AND_SEEK = 3U;

// --- Callbacks Support ---

typedef struct {
    // Returns number of bytes read, 0 on EOF
    size_t (*read)(void* user_data, char* dst, size_t len);
    // Returns new offset, or current offset if origin is SEEK_CUR with 0 offset
    uint64_t (*seek)(void* user_data, int64_t offset, int origin);
    // Returns total size if known, or UINT64_MAX when the size is unknown or unavailable.
    uint64_t (*get_size)(void* user_data);
    // Create a clone of user_data. If null, parallelism will be limited to 1
    void* (*clone)(void* user_data);
    // Free the user_data when the reader is closed
    void (*free_user_data)(void* user_data);
} rgz_callbacks_t;

class CallbackFileReader : public rapidgzip::FileReader {
public:
    CallbackFileReader(rgz_callbacks_t callbacks, void* user_data) 
        : m_callbacks(callbacks), m_user_data(user_data), m_eof(false) {}

    ~CallbackFileReader() override {
        close();
    }

    void close() override {
        if (m_callbacks.free_user_data && m_user_data) {
            m_callbacks.free_user_data(m_user_data);
        }
        m_user_data = nullptr;
    }
    bool closed() const override { return m_user_data == nullptr; }
    bool eof() const override { return m_eof; }
    bool fail() const override { return false; }
    int fileno() const override { return -1; }
    bool seekable() const override { return m_callbacks.seek != nullptr; }

    size_t read(char* buffer, size_t nMaxBytesToRead) override {
        if (!m_user_data || !m_callbacks.read) return 0;
        size_t n = m_callbacks.read(m_user_data, buffer, nMaxBytesToRead);
        
        if (n == static_cast<size_t>(-1)) {
            throw std::runtime_error("IO error during read callback in Rust");
        }
        
        if (n == 0 && nMaxBytesToRead > 0) m_eof = true;
        return n;
    }

    size_t seek(long long int offset, int origin) override {
        if (!m_user_data || !m_callbacks.seek) return tell();
        m_eof = false;
        uint64_t res = m_callbacks.seek(m_user_data, offset, origin);
        
        if (res == static_cast<uint64_t>(-1)) {
            throw std::runtime_error("IO error during seek callback in Rust");
        }
        return static_cast<size_t>(res);
    }

    size_t tell() const override {
        if (!m_user_data || !m_callbacks.seek) return 0;
        uint64_t res = m_callbacks.seek(m_user_data, 0, SEEK_CUR);
        if (res == static_cast<uint64_t>(-1)) {
            throw std::runtime_error("IO error during tell callback in Rust");
        }
        return static_cast<size_t>(res);
    }

    std::optional<size_t> size() const override {
        if (!m_user_data || !m_callbacks.get_size) return std::nullopt;
        uint64_t s = m_callbacks.get_size(m_user_data);
        if (s == std::numeric_limits<uint64_t>::max()) return std::nullopt;
        return std::make_optional(static_cast<size_t>(s));
    }

    void clearerr() override { m_eof = false; }

    // Clone support for ParallelGzipReader
    std::unique_ptr<rapidgzip::FileReader> cloneRaw() const override {
        if (!m_callbacks.clone || !m_user_data) {
            throw std::logic_error("Cloning CallbackFileReader is not supported because no clone callback was provided");
        }
        void* cloned_user_data = m_callbacks.clone(m_user_data);
        if (!cloned_user_data) {
            throw std::runtime_error("Clone callback returned NULL");
        }
        std::unique_ptr<void, void(*)(void*)> guard(
            cloned_user_data, m_callbacks.free_user_data ? m_callbacks.free_user_data : [](void*){}
        );
        auto reader = std::make_unique<CallbackFileReader>(m_callbacks, cloned_user_data);
        guard.release();
        return reader;
    }

private:
    rgz_callbacks_t m_callbacks;
    void* m_user_data;
    bool m_eof;
};

// --- End Callbacks Support ---

// Opaque reader handle
struct rgz_reader_t {
    std::unique_ptr<rapidgzip::ParallelGzipReader<>> reader;
    std::string last_error;
    rgz_status_t last_status;

    rgz_reader_t() : last_status(RGZ_STATUS_OK) {}
};

namespace {

thread_local std::string g_last_global_error;

void clear_global_error() {
    g_last_global_error.clear();
}

void set_global_error(const std::string& message) {
    g_last_global_error = message;
}

}  // namespace

namespace {

rapidgzip::UniqueFileReader wrap_file_reader_for_config(rapidgzip::UniqueFileReader&& file_reader, uint32_t flags)
{
    const auto io_read_mode = flags & RGZ_IO_READ_MODE_MASK;

    // Expose the upstream I/O strategies without changing the Rust-visible reader API shape.
    switch (io_read_mode)
    {
    case RGZ_IO_READ_MODE_SEQUENTIAL:
        return std::make_unique<rapidgzip::SinglePassFileReader>(std::move(file_reader));

    case RGZ_IO_READ_MODE_PREAD:
    case RGZ_IO_READ_MODE_LOCKED_READ_AND_SEEK:
    {
        auto shared_file = rapidgzip::ensureSharedFileReader(std::move(file_reader));
        shared_file->setUsePread(io_read_mode == RGZ_IO_READ_MODE_PREAD);
        return shared_file;
    }

    case RGZ_IO_READ_MODE_AUTO:
    default:
        return std::move(file_reader);
    }
}

}  // namespace

uint32_t rgz_abi_version(void) {
    return (RGZ_ABI_VERSION_MAJOR << 16) | RGZ_ABI_VERSION_MINOR;
}

void rgz_config_init(rgz_config_t* config) {
    if (!config) return;
    std::memset(config, 0, sizeof(rgz_config_t));
    config->struct_size = sizeof(rgz_config_t);
}

rgz_reader_t* rgz_open_path_ex(
    const char* path,
    const rgz_config_t* config,
    rgz_status_t* status_out
) {
    clear_global_error();
    if (!path) {
        set_global_error("Path pointer must not be null");
        if (status_out) *status_out = RGZ_STATUS_INVALID_ARGUMENT;
        return nullptr;
    }

    try {
        auto rgz_reader = std::make_unique<rgz_reader_t>();
        
        uint32_t parallelism = 0;
        uint64_t chunk_size = 4 * 1024 * 1024; // 4MiB default

        if (config) {
            if (config->struct_size < sizeof(rgz_config_t)) {
                set_global_error("rgz_config_t is smaller than the supported ABI");
                if (status_out) *status_out = RGZ_STATUS_INVALID_ARGUMENT;
                return nullptr;
            }
            parallelism = config->parallelism;
            if (config->chunk_size > 0) {
                chunk_size = config->chunk_size;
            }
        }

        auto file_reader = std::make_unique<rapidgzip::StandardFileReader>(path);
        auto configured_reader = wrap_file_reader_for_config(std::move(file_reader), config ? config->flags : 0);
        rgz_reader->reader = std::make_unique<rapidgzip::ParallelGzipReader<>>(
            std::move(configured_reader), parallelism, chunk_size
        );

        clear_global_error();
        if (status_out) *status_out = RGZ_STATUS_OK;
        return rgz_reader.release();
    } catch (const std::exception& e) {
        set_global_error(e.what());
        if (status_out) *status_out = RGZ_STATUS_IO_ERROR;
        return nullptr;
    } catch (...) {
        set_global_error("Unknown C++ exception while opening a filesystem path");
        if (status_out) *status_out = RGZ_STATUS_INTERNAL_ERROR;
        return nullptr;
    }
}

rgz_reader_t* rgz_open_callbacks(
    rgz_callbacks_t callbacks,
    void* user_data,
    const rgz_config_t* config,
    rgz_status_t* status_out
) {
    clear_global_error();
    std::unique_ptr<void, void(*)(void*)> guard(
        user_data, callbacks.free_user_data ? callbacks.free_user_data : [](void*){}
    );

    try {
        auto rgz_reader = std::make_unique<rgz_reader_t>();
        
        uint32_t parallelism = 0; // Default: use rapidgzip policy
        uint64_t chunk_size = 4 * 1024 * 1024;

        if (config) {
            parallelism = config->parallelism;
            if (config->chunk_size > 0) chunk_size = config->chunk_size;
        }

        // If cloning is not supported, we MUST force parallelism to 1
        if (parallelism != 1 && !callbacks.clone) {
             parallelism = 1; 
        }

        auto file_reader = std::make_unique<CallbackFileReader>(callbacks, user_data);
        guard.release(); // Ownership is now successfully managed by CallbackFileReader

        auto configured_reader = wrap_file_reader_for_config(std::move(file_reader), config ? config->flags : 0);
        rgz_reader->reader = std::make_unique<rapidgzip::ParallelGzipReader<>>(
            std::move(configured_reader), parallelism, chunk_size
        );

        clear_global_error();
        if (status_out) *status_out = RGZ_STATUS_OK;
        return rgz_reader.release();
    } catch (const std::exception& e) {
        set_global_error(e.what());
        if (status_out) *status_out = RGZ_STATUS_IO_ERROR;
        return nullptr;
    } catch (...) {
        set_global_error("Unknown C++ exception while opening callback-backed input");
        if (status_out) *status_out = RGZ_STATUS_INTERNAL_ERROR;
        return nullptr;
    }
}

void rgz_close(rgz_reader_t* reader) {
    if (!reader) {
        return;
    }

    try {
        if (reader->reader) {
            reader->reader->close();
        }
    } catch (...) {
    }

    delete reader;
}

namespace {

rgz_status_t finish_read_status(rgz_reader_t* reader, size_t len, size_t n) {
    if (n == 0 && len > 0) {
        reader->last_status = RGZ_STATUS_EOF;
        return RGZ_STATUS_EOF;
    }

    reader->last_status = RGZ_STATUS_OK;
    return RGZ_STATUS_OK;
}

}  // namespace

rgz_status_t rgz_read(
    rgz_reader_t* reader,
    void* dst,
    size_t len,
    size_t* read_out
) {
    if (!reader || !reader->reader) return RGZ_STATUS_INVALID_ARGUMENT;
    if (len > 0 && !dst) return RGZ_STATUS_INVALID_ARGUMENT;
    if (!read_out) return RGZ_STATUS_INVALID_ARGUMENT;

    try {
        size_t n = reader->reader->read(reinterpret_cast<char*>(dst), len);
        *read_out = n;
        return finish_read_status(reader, len, n);
    } catch (const std::exception& e) {
        reader->last_error = e.what();
        reader->last_status = RGZ_STATUS_DATA_ERROR;
        return RGZ_STATUS_DATA_ERROR;
    } catch (...) {
        reader->last_error = "Unknown C++ exception";
        reader->last_status = RGZ_STATUS_INTERNAL_ERROR;
        return RGZ_STATUS_INTERNAL_ERROR;
    }
}

rgz_status_t rgz_read_discard(
    rgz_reader_t* reader,
    size_t len,
    size_t* read_out
) {
    if (!reader || !reader->reader) return RGZ_STATUS_INVALID_ARGUMENT;
    if (!read_out) return RGZ_STATUS_INVALID_ARGUMENT;

    try {
        size_t n = reader->reader->read(-1, nullptr, len);
        *read_out = n;
        return finish_read_status(reader, len, n);
    } catch (const std::exception& e) {
        reader->last_error = e.what();
        reader->last_status = RGZ_STATUS_DATA_ERROR;
        return RGZ_STATUS_DATA_ERROR;
    } catch (...) {
        reader->last_error = "Unknown C++ exception";
        reader->last_status = RGZ_STATUS_INTERNAL_ERROR;
        return RGZ_STATUS_INTERNAL_ERROR;
    }
}

rgz_status_t rgz_read_to_fd(
    rgz_reader_t* reader,
    int output_fd,
    size_t len,
    size_t* read_out
) {
    if (!reader || !reader->reader) return RGZ_STATUS_INVALID_ARGUMENT;
    if (!read_out || output_fd < 0) return RGZ_STATUS_INVALID_ARGUMENT;

    try {
        size_t n = reader->reader->read(output_fd, nullptr, len);
        *read_out = n;
        return finish_read_status(reader, len, n);
    } catch (const std::exception& e) {
        reader->last_error = e.what();
        reader->last_status = RGZ_STATUS_DATA_ERROR;
        return RGZ_STATUS_DATA_ERROR;
    } catch (...) {
        reader->last_error = "Unknown C++ exception";
        reader->last_status = RGZ_STATUS_INTERNAL_ERROR;
        return RGZ_STATUS_INTERNAL_ERROR;
    }
}

rgz_status_t rgz_set_keep_index(
    rgz_reader_t* reader,
    bool keep_index
) {
    if (!reader || !reader->reader) return RGZ_STATUS_INVALID_ARGUMENT;

    try {
        reader->reader->setKeepIndex(keep_index);
        reader->last_status = RGZ_STATUS_OK;
        return RGZ_STATUS_OK;
    } catch (const std::exception& e) {
        reader->last_error = e.what();
        reader->last_status = RGZ_STATUS_STATE_ERROR;
        return RGZ_STATUS_STATE_ERROR;
    } catch (...) {
        reader->last_error = "Unknown C++ exception";
        reader->last_status = RGZ_STATUS_INTERNAL_ERROR;
        return RGZ_STATUS_INTERNAL_ERROR;
    }
}

rgz_status_t rgz_seek_to(
    rgz_reader_t* reader,
    uint64_t uncompressed_offset
) {
    if (!reader || !reader->reader) return RGZ_STATUS_INVALID_ARGUMENT;

    try {
        reader->reader->seek(uncompressed_offset, 0);
        reader->last_status = RGZ_STATUS_OK;
        return RGZ_STATUS_OK;
    } catch (const std::exception& e) {
        reader->last_error = e.what();
        reader->last_status = RGZ_STATUS_SEEK_ERROR;
        return RGZ_STATUS_SEEK_ERROR;
    } catch (...) {
        reader->last_error = "Unknown C++ exception";
        reader->last_status = RGZ_STATUS_INTERNAL_ERROR;
        return RGZ_STATUS_INTERNAL_ERROR;
    }
}

rgz_status_t rgz_tell(
    rgz_reader_t* reader,
    uint64_t* offset_out
) {
    if (!reader || !reader->reader || !offset_out) return RGZ_STATUS_INVALID_ARGUMENT;

    try {
        *offset_out = reader->reader->tell();
        reader->last_status = RGZ_STATUS_OK;
        return RGZ_STATUS_OK;
    } catch (const std::exception& e) {
        reader->last_error = e.what();
        reader->last_status = RGZ_STATUS_INTERNAL_ERROR;
        return RGZ_STATUS_INTERNAL_ERROR;
    }
}

rgz_status_t rgz_import_index_path(
    rgz_reader_t* reader,
    const char* path
) {
    if (!reader || !reader->reader || !path) return RGZ_STATUS_INVALID_ARGUMENT;

    try {
        auto index_file = std::make_unique<rapidgzip::StandardFileReader>(path);
        reader->reader->importIndex(std::move(index_file));
        reader->last_status = RGZ_STATUS_OK;
        return RGZ_STATUS_OK;
    } catch (const std::exception& e) {
        reader->last_error = e.what();
        reader->last_status = RGZ_STATUS_INDEX_ERROR;
        return RGZ_STATUS_INDEX_ERROR;
    } catch (...) {
        reader->last_error = "Unknown C++ exception during index import";
        reader->last_status = RGZ_STATUS_INTERNAL_ERROR;
        return RGZ_STATUS_INTERNAL_ERROR;
    }
}

rgz_status_t rgz_export_index_path(
    rgz_reader_t* reader,
    const char* path
) {
    if (!reader || !reader->reader || !path) return RGZ_STATUS_INVALID_ARGUMENT;

    try {
        std::ofstream out(path, std::ios::binary);
        if (!out) {
            reader->last_error = "Failed to open index file for writing";
            reader->last_status = RGZ_STATUS_IO_ERROR;
            return RGZ_STATUS_IO_ERROR;
        }

        auto checkedWrite = [&out](const void* buffer, size_t size) {
            out.write(reinterpret_cast<const char*>(buffer), size);
            if (!out) {
                throw std::runtime_error("Failed to write to index file");
            }
        };

        reader->reader->exportIndex(checkedWrite);
        reader->last_status = RGZ_STATUS_OK;
        return RGZ_STATUS_OK;
    } catch (const std::exception& e) {
        reader->last_error = e.what();
        reader->last_status = RGZ_STATUS_INDEX_ERROR;
        return RGZ_STATUS_INDEX_ERROR;
    } catch (...) {
        reader->last_error = "Unknown C++ exception during index export";
        reader->last_status = RGZ_STATUS_INTERNAL_ERROR;
        return RGZ_STATUS_INTERNAL_ERROR;
    }
}

rgz_status_t rgz_import_index_callbacks(
    rgz_reader_t* reader,
    rgz_callbacks_t callbacks,
    void* user_data
) {
    std::unique_ptr<void, void(*)(void*)> guard(
        user_data, callbacks.free_user_data ? callbacks.free_user_data : [](void*){}
    );

    if (!reader || !reader->reader) return RGZ_STATUS_INVALID_ARGUMENT;

    try {
        auto index_file = std::make_unique<CallbackFileReader>(callbacks, user_data);
        guard.release();
        reader->reader->importIndex(std::move(index_file));
        reader->last_status = RGZ_STATUS_OK;
        return RGZ_STATUS_OK;
    } catch (const std::exception& e) {
        reader->last_error = e.what();
        reader->last_status = RGZ_STATUS_INDEX_ERROR;
        return RGZ_STATUS_INDEX_ERROR;
    } catch (...) {
        reader->last_error = "Unknown C++ exception during index import";
        reader->last_status = RGZ_STATUS_INTERNAL_ERROR;
        return RGZ_STATUS_INTERNAL_ERROR;
    }
}

rgz_status_t rgz_export_index_write(
    rgz_reader_t* reader,
    size_t (*write_cb)(void* user_data, const void* buffer, size_t size),
    void* user_data
) {
    if (!reader || !reader->reader || !write_cb) return RGZ_STATUS_INVALID_ARGUMENT;

    try {
        auto checkedWrite = [write_cb, user_data](const void* buffer, size_t size) {
            size_t written = write_cb(user_data, buffer, size);
            if (written != size) {
                throw std::runtime_error("Failed to write to index file via callback");
            }
        };

        reader->reader->exportIndex(checkedWrite);
        reader->last_status = RGZ_STATUS_OK;
        return RGZ_STATUS_OK;
    } catch (const std::exception& e) {
        reader->last_error = e.what();
        reader->last_status = RGZ_STATUS_INDEX_ERROR;
        return RGZ_STATUS_INDEX_ERROR;
    } catch (...) {
        reader->last_error = "Unknown C++ exception during index export";
        reader->last_status = RGZ_STATUS_INTERNAL_ERROR;
        return RGZ_STATUS_INTERNAL_ERROR;
    }
}

const char* rgz_last_error(rgz_reader_t* reader) {
    if (!reader) return "Null reader handle";
    return reader->last_error.c_str();
}

const char* rgz_last_global_error(void) {
    if (g_last_global_error.empty()) return nullptr;
    return g_last_global_error.c_str();
}

rgz_status_t rgz_last_status(rgz_reader_t* reader) {
    if (!reader) return RGZ_STATUS_INVALID_ARGUMENT;
    return reader->last_status;
}

} // extern "C"
