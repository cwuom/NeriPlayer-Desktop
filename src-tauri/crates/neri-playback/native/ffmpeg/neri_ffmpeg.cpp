// FFmpeg 运行时加载垫片
//
// 按 include/ 下固定主版本的头文件编译，运行时从指定目录加载 avutil/avcodec/avformat。
// 只向 Rust 暴露一组稳定的 C 接口；FFmpeg 的结构体字段只在这里访问，偏移由编译器按头文件保证。
// 这里不抛异常、不分配 C++ 异常对象：所有失败都以负返回值加错误文本交回调用方。

#ifndef __STDC_CONSTANT_MACROS
#define __STDC_CONSTANT_MACROS
#endif

extern "C" {
#include <libavcodec/avcodec.h>
#include <libavformat/avformat.h>
#include <libavutil/avutil.h>
#include <libavutil/channel_layout.h>
#include <libavutil/dict.h>
#include <libavutil/frame.h>
#include <libavutil/log.h>
#include <libavutil/mathematics.h>
#include <libavutil/mem.h>
#include <libavutil/samplefmt.h>
}

#include <atomic>
#include <cmath>
#include <cstdarg>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <mutex>
#include <new>

#ifdef _WIN32
#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>
#else
#include <dlfcn.h>
#endif

#define NERI_FF_AVUTIL(X)                                                                  \
    X(avutil_version) X(av_malloc) X(av_free) X(av_frame_alloc) X(av_frame_free)           \
    X(av_frame_unref) X(av_dict_set) X(av_dict_get) X(av_dict_free) X(av_strerror)         \
    X(av_log_set_level) X(av_log_set_callback) X(av_log_format_line2)                      \
    X(av_channel_layout_channel_from_index) X(av_channel_layout_default)                   \
    X(av_channel_layout_uninit) X(av_rescale_q)

#define NERI_FF_AVCODEC(X)                                                                 \
    X(avcodec_version) X(avcodec_find_decoder_by_name) X(avcodec_alloc_context3)           \
    X(avcodec_free_context) X(avcodec_parameters_to_context) X(avcodec_open2)              \
    X(avcodec_send_packet) X(avcodec_receive_frame) X(avcodec_flush_buffers)               \
    X(avcodec_get_name) X(av_packet_alloc) X(av_packet_free) X(av_packet_unref)

#define NERI_FF_AVFORMAT(X)                                                                \
    X(avformat_version) X(avformat_alloc_context) X(avformat_open_input)                   \
    X(avformat_find_stream_info) X(av_find_best_stream) X(av_read_frame) X(av_seek_frame)  \
    X(avformat_close_input) X(avio_alloc_context) X(avio_context_free) X(av_find_input_format)

namespace {

struct FfApi {
#define NERI_FF_FIELD(name) decltype(&::name) name;
    NERI_FF_AVUTIL(NERI_FF_FIELD)
    NERI_FF_AVCODEC(NERI_FF_FIELD)
    NERI_FF_AVFORMAT(NERI_FF_FIELD)
#undef NERI_FF_FIELD
};

#define NERI_STR2(value) #value
#define NERI_STR(value) NERI_STR2(value)

#if defined(_WIN32)
using LibHandle = HMODULE;
const char* const kLibraryNames[3] = {
    "avutil-" NERI_STR(LIBAVUTIL_VERSION_MAJOR) ".dll",
    "avcodec-" NERI_STR(LIBAVCODEC_VERSION_MAJOR) ".dll",
    "avformat-" NERI_STR(LIBAVFORMAT_VERSION_MAJOR) ".dll",
};
#elif defined(__APPLE__)
using LibHandle = void*;
const char* const kLibraryNames[3] = {
    "libavutil." NERI_STR(LIBAVUTIL_VERSION_MAJOR) ".dylib",
    "libavcodec." NERI_STR(LIBAVCODEC_VERSION_MAJOR) ".dylib",
    "libavformat." NERI_STR(LIBAVFORMAT_VERSION_MAJOR) ".dylib",
};
#else
using LibHandle = void*;
const char* const kLibraryNames[3] = {
    "libavutil.so." NERI_STR(LIBAVUTIL_VERSION_MAJOR),
    "libavcodec.so." NERI_STR(LIBAVCODEC_VERSION_MAJOR),
    "libavformat.so." NERI_STR(LIBAVFORMAT_VERSION_MAJOR),
};
#endif

FfApi api{};
LibHandle libraries[3] = {};
std::mutex load_mutex;
std::atomic<int> load_state{0};  // 0 未加载，1 已加载，-1 最近一次失败
char load_error[512] = {0};
std::atomic<void (*)(int32_t, const char*)> log_sink{nullptr};

constexpr int kMaxChannels = 64;
constexpr int kIoBufferSize = 64 * 1024;
constexpr int kMaxConsecutiveDecodeErrors = 64;
constexpr int32_t kIoInterrupted = -2;

LibHandle open_library(const char* directory, const char* name, char* error, size_t error_len) {
    char path[4096];
#ifdef _WIN32
    int written = std::snprintf(path, sizeof(path), "%s\\%s", directory, name);
#else
    int written = std::snprintf(path, sizeof(path), "%s/%s", directory, name);
#endif
    if (written <= 0 || written >= static_cast<int>(sizeof(path))) {
        std::snprintf(error, error_len, "library path is too long: %s", name);
        return nullptr;
    }
#ifdef _WIN32
    wchar_t wide[4096];
    if (MultiByteToWideChar(CP_UTF8, 0, path, -1, wide, 4096) == 0) {
        std::snprintf(error, error_len, "library path is not valid UTF-8: %s", name);
        return nullptr;
    }
    // 只在 DLL 所在目录与系统目录里找依赖，避免当前目录或 PATH 上的同名 DLL 被劫持加载
    HMODULE handle = LoadLibraryExW(
        wide, nullptr, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS);
    if (handle == nullptr) {
        std::snprintf(error, error_len, "could not load %s (Windows error %lu)", path,
                      static_cast<unsigned long>(GetLastError()));
    }
    return handle;
#else
    void* handle = dlopen(path, RTLD_NOW | RTLD_LOCAL);
    if (handle == nullptr) {
        const char* reason = dlerror();
        std::snprintf(error, error_len, "could not load %s (%s)", path, reason ? reason : "unknown");
    }
    return handle;
#endif
}

void* find_symbol(LibHandle handle, const char* name) {
#ifdef _WIN32
    return reinterpret_cast<void*>(GetProcAddress(handle, name));
#else
    return dlsym(handle, name);
#endif
}

void close_library(LibHandle handle) {
    if (handle == nullptr) return;
#ifdef _WIN32
    FreeLibrary(handle);
#else
    dlclose(handle);
#endif
}

// 主版本必须一致；次版本不能低于编译时的头文件，否则结构体尾部新增的字段在运行时并不存在
bool version_compatible(unsigned runtime, int header_major, int header_minor) {
    return static_cast<int>(AV_VERSION_MAJOR(runtime)) == header_major &&
           static_cast<int>(AV_VERSION_MINOR(runtime)) >= header_minor;
}

void forward_log(void* context, int level, const char* format, va_list arguments) {
    if (level > AV_LOG_WARNING) return;
    auto sink = log_sink.load(std::memory_order_acquire);
    if (sink == nullptr || api.av_log_format_line2 == nullptr) return;
    char line[1024];
    int print_prefix = 1;
    va_list copy;
    va_copy(copy, arguments);
    api.av_log_format_line2(context, level, format, copy, line, sizeof(line), &print_prefix);
    va_end(copy);
    size_t length = std::strlen(line);
    while (length > 0 && (line[length - 1] == '\n' || line[length - 1] == '\r')) {
        line[--length] = '\0';
    }
    if (length > 0) sink(level, line);
}

}  // namespace

extern "C" {

struct NeriFfIo {
    void* opaque;
    // 返回读到的字节数；0 表示结束；kIoInterrupted 表示调用方取消；其他负数表示 I/O 错误
    int32_t (*read)(void* opaque, uint8_t* buffer, int32_t size);
    // whence 为 SEEK_SET/SEEK_CUR/SEEK_END，或 0x10000 询问总长度；负数表示失败或未知
    int64_t (*seek)(void* opaque, int64_t offset, int32_t whence);
    int32_t (*interrupted)(void* opaque);
};

struct NeriFfStreamInfo {
    int32_t sample_rate;
    int32_t source_channels;
    int32_t output_channels;
    int32_t bits_per_sample;
    int64_t bit_rate;
    int64_t duration_us;
    int32_t initial_padding;
    int32_t trailing_padding;
    int32_t seek_preroll;
    int32_t decoder_downmix;
    int32_t matrix_downmix;
    char codec[32];
    char container[64];
};

struct NeriFfDecoder {
    NeriFfIo io{};
    AVFormatContext* format = nullptr;
    AVIOContext* avio = nullptr;
    AVCodecContext* codec = nullptr;
    AVPacket* packet = nullptr;
    AVFrame* frame = nullptr;
    int stream_index = -1;
    AVRational time_base{1, 1};
    int64_t stream_start = 0;
    int sample_rate = 0;
    int max_output_channels = 2;
    int frame_channels = 0;
    int output_channels = 0;
    bool use_matrix = false;
    float matrix[2][kMaxChannels] = {};
    bool frame_ready = false;
    int frame_offset = 0;
    bool input_finished = false;
    int64_t discard_before = -1;
    int consecutive_errors = 0;
    char error[256] = {0};
    NeriFfStreamInfo info{};
};

}  // extern "C"

namespace {

int io_read(void* opaque, uint8_t* buffer, int size) {
    auto* decoder = static_cast<NeriFfDecoder*>(opaque);
    int32_t result = decoder->io.read(decoder->io.opaque, buffer, size);
    if (result > 0) return result;
    if (result == 0) return AVERROR_EOF;
    return result == kIoInterrupted ? AVERROR_EXIT : AVERROR(EIO);
}

int64_t io_seek(void* opaque, int64_t offset, int whence) {
    auto* decoder = static_cast<NeriFfDecoder*>(opaque);
    int64_t result = decoder->io.seek(decoder->io.opaque, offset, (whence & ~AVSEEK_FORCE));
    return result < 0 ? AVERROR(EIO) : result;
}

int io_interrupted(void* opaque) {
    auto* decoder = static_cast<NeriFfDecoder*>(opaque);
    return decoder->io.interrupted != nullptr && decoder->io.interrupted(decoder->io.opaque) != 0;
}

int fail(NeriFfDecoder* decoder, const char* what, int code) {
    char reason[128] = {0};
    if (code < 0 && api.av_strerror != nullptr && api.av_strerror(code, reason, sizeof(reason)) < 0) {
        std::snprintf(reason, sizeof(reason), "error %d", code);
    }
    if (reason[0] != '\0') {
        std::snprintf(decoder->error, sizeof(decoder->error), "%s: %s", what, reason);
    } else {
        std::snprintf(decoder->error, sizeof(decoder->error), "%s", what);
    }
    return code < 0 ? code : AVERROR_UNKNOWN;
}

void destroy(NeriFfDecoder* decoder) {
    if (decoder == nullptr) return;
    if (decoder->frame != nullptr) api.av_frame_free(&decoder->frame);
    if (decoder->packet != nullptr) api.av_packet_free(&decoder->packet);
    if (decoder->codec != nullptr) api.avcodec_free_context(&decoder->codec);
    if (decoder->format != nullptr) api.avformat_close_input(&decoder->format);
    if (decoder->avio != nullptr) {
        // 自定义 I/O 的缓冲可能被 FFmpeg 换过，必须从上下文里取当前那块释放
        api.av_free(decoder->avio->buffer);
        api.avio_context_free(&decoder->avio);
    }
    delete decoder;
}

// 读包、送解码器、取帧；返回 1 取到帧，0 已经解完，负数为错误
int receive_frame(NeriFfDecoder* decoder) {
    for (;;) {
        int result = api.avcodec_receive_frame(decoder->codec, decoder->frame);
        if (result == 0) return 1;
        if (result == AVERROR_EOF) return 0;
        if (result != AVERROR(EAGAIN)) return fail(decoder, "decoding failed", result);
        if (decoder->input_finished) return 0;

        result = api.av_read_frame(decoder->format, decoder->packet);
        if (result == AVERROR_EOF) {
            decoder->input_finished = true;
            api.avcodec_send_packet(decoder->codec, nullptr);
            continue;
        }
        if (result < 0) {
            return fail(decoder, io_interrupted(decoder) ? "reading was interrupted" : "reading failed",
                        result);
        }
        if (decoder->packet->stream_index != decoder->stream_index) {
            api.av_packet_unref(decoder->packet);
            continue;
        }
        result = api.avcodec_send_packet(decoder->codec, decoder->packet);
        api.av_packet_unref(decoder->packet);
        if (result < 0 && result != AVERROR(EAGAIN)) {
            // 坏包跳过继续解：网络流偶发的损坏帧不应当整首失败，但连续坏包说明流本身不可解
            if (++decoder->consecutive_errors > kMaxConsecutiveDecodeErrors) {
                return fail(decoder, "too many undecodable packets", result);
            }
            continue;
        }
        decoder->consecutive_errors = 0;
    }
}

void channel_weights(AVChannel channel, float* left, float* right) {
    constexpr float kSide = 0.70710678f;
    switch (channel) {
        case AV_CHAN_FRONT_LEFT:
        case AV_CHAN_STEREO_LEFT:
            *left = 1.0f; *right = 0.0f; return;
        case AV_CHAN_FRONT_RIGHT:
        case AV_CHAN_STEREO_RIGHT:
            *left = 0.0f; *right = 1.0f; return;
        case AV_CHAN_FRONT_CENTER:
            *left = kSide; *right = kSide; return;
        case AV_CHAN_LOW_FREQUENCY:
        case AV_CHAN_LOW_FREQUENCY_2:
            // ITU-R BS.775 默认下混不计入低频声道
            *left = 0.0f; *right = 0.0f; return;
        case AV_CHAN_BACK_LEFT:
        case AV_CHAN_SIDE_LEFT:
        case AV_CHAN_FRONT_LEFT_OF_CENTER:
        case AV_CHAN_WIDE_LEFT:
        case AV_CHAN_SURROUND_DIRECT_LEFT:
        case AV_CHAN_TOP_FRONT_LEFT:
        case AV_CHAN_TOP_BACK_LEFT:
        case AV_CHAN_TOP_SIDE_LEFT:
        case AV_CHAN_BOTTOM_FRONT_LEFT:
            *left = kSide; *right = 0.0f; return;
        case AV_CHAN_BACK_RIGHT:
        case AV_CHAN_SIDE_RIGHT:
        case AV_CHAN_FRONT_RIGHT_OF_CENTER:
        case AV_CHAN_WIDE_RIGHT:
        case AV_CHAN_SURROUND_DIRECT_RIGHT:
        case AV_CHAN_TOP_FRONT_RIGHT:
        case AV_CHAN_TOP_BACK_RIGHT:
        case AV_CHAN_TOP_SIDE_RIGHT:
        case AV_CHAN_BOTTOM_FRONT_RIGHT:
            *left = 0.0f; *right = kSide; return;
        default:
            *left = 0.5f; *right = 0.5f; return;
    }
}

// 码流不带下混元数据时按 ITU 系数下混，再整体缩放到各输出的系数和不超过 1，避免削波
void build_downmix_matrix(NeriFfDecoder* decoder, const AVChannelLayout& source) {
    AVChannelLayout fallback{};
    const AVChannelLayout* layout = &source;
    bool owns_fallback = false;
    if (source.order == AV_CHANNEL_ORDER_UNSPEC) {
        api.av_channel_layout_default(&fallback, source.nb_channels);
        layout = &fallback;
        owns_fallback = true;
    }
    const int channels = source.nb_channels < kMaxChannels ? source.nb_channels : kMaxChannels;
    std::memset(decoder->matrix, 0, sizeof(decoder->matrix));
    for (int index = 0; index < channels; ++index) {
        float left = 0.0f;
        float right = 0.0f;
        channel_weights(api.av_channel_layout_channel_from_index(layout, static_cast<unsigned>(index)),
                        &left, &right);
        if (decoder->output_channels == 1) {
            // 单声道取立体声下混的平均
            decoder->matrix[0][index] = (left + right) * 0.5f;
        } else {
            decoder->matrix[0][index] = left;
            decoder->matrix[1][index] = right;
        }
    }
    float largest = 0.0f;
    for (int output = 0; output < decoder->output_channels; ++output) {
        float sum = 0.0f;
        for (int index = 0; index < channels; ++index) sum += std::fabs(decoder->matrix[output][index]);
        if (sum > largest) largest = sum;
    }
    if (largest > 1.0f) {
        for (int output = 0; output < decoder->output_channels; ++output) {
            for (int index = 0; index < channels; ++index) decoder->matrix[output][index] /= largest;
        }
    }
    if (owns_fallback) api.av_channel_layout_uninit(&fallback);
}

void configure_channels(NeriFfDecoder* decoder) {
    const int channels = decoder->frame->ch_layout.nb_channels;
    decoder->frame_channels = channels;
    decoder->output_channels = channels > decoder->max_output_channels ? decoder->max_output_channels : channels;
    decoder->use_matrix = channels > decoder->output_channels;
    if (decoder->use_matrix) build_downmix_matrix(decoder, decoder->frame->ch_layout);
}

inline float sample_at(const AVFrame* frame, int channels, int channel, int index) {
    const uint8_t* const* planes = frame->extended_data;
    switch (frame->format) {
        case AV_SAMPLE_FMT_U8:
            return (static_cast<int>(planes[0][index * channels + channel]) - 128) / 128.0f;
        case AV_SAMPLE_FMT_S16:
            return reinterpret_cast<const int16_t*>(planes[0])[index * channels + channel] / 32768.0f;
        case AV_SAMPLE_FMT_S32:
            return static_cast<float>(reinterpret_cast<const int32_t*>(planes[0])[index * channels + channel] / 2147483648.0);
        case AV_SAMPLE_FMT_FLT:
            return reinterpret_cast<const float*>(planes[0])[index * channels + channel];
        case AV_SAMPLE_FMT_DBL:
            return static_cast<float>(reinterpret_cast<const double*>(planes[0])[index * channels + channel]);
        case AV_SAMPLE_FMT_S64:
            return static_cast<float>(reinterpret_cast<const int64_t*>(planes[0])[index * channels + channel] / 9223372036854775808.0);
        case AV_SAMPLE_FMT_U8P:
            return (static_cast<int>(planes[channel][index]) - 128) / 128.0f;
        case AV_SAMPLE_FMT_S16P:
            return reinterpret_cast<const int16_t*>(planes[channel])[index] / 32768.0f;
        case AV_SAMPLE_FMT_S32P:
            return static_cast<float>(reinterpret_cast<const int32_t*>(planes[channel])[index] / 2147483648.0);
        case AV_SAMPLE_FMT_FLTP:
            return reinterpret_cast<const float*>(planes[channel])[index];
        case AV_SAMPLE_FMT_DBLP:
            return static_cast<float>(reinterpret_cast<const double*>(planes[channel])[index]);
        case AV_SAMPLE_FMT_S64P:
            return static_cast<float>(reinterpret_cast<const int64_t*>(planes[channel])[index] / 9223372036854775808.0);
        default:
            return 0.0f;
    }
}

int64_t frame_start_sample(const NeriFfDecoder* decoder) {
    int64_t pts = decoder->frame->best_effort_timestamp;
    if (pts == AV_NOPTS_VALUE) pts = decoder->frame->pts;
    if (pts == AV_NOPTS_VALUE || decoder->sample_rate <= 0) return AV_NOPTS_VALUE;
    return api.av_rescale_q(pts - decoder->stream_start, decoder->time_base, AVRational{1, decoder->sample_rate});
}

// 取下一块可输出的帧：处理 seek 之后的精确丢样，返回 1 有帧、0 结束、负数错误
int next_ready_frame(NeriFfDecoder* decoder) {
    for (;;) {
        int result = receive_frame(decoder);
        if (result <= 0) return result;
        decoder->frame_offset = 0;
        if (decoder->frame->ch_layout.nb_channels != decoder->frame_channels) configure_channels(decoder);
        if (decoder->discard_before >= 0) {
            const int64_t start = frame_start_sample(decoder);
            if (start == AV_NOPTS_VALUE) {
                // 没有时间戳就无法精确定位，只能从解封装器的落点开始
                decoder->discard_before = -1;
            } else {
                const int64_t end = start + decoder->frame->nb_samples;
                if (end <= decoder->discard_before) {
                    api.av_frame_unref(decoder->frame);
                    continue;
                }
                if (start < decoder->discard_before) {
                    decoder->frame_offset = static_cast<int>(decoder->discard_before - start);
                }
                decoder->discard_before = -1;
            }
        }
        return 1;
    }
}

void copy_cstr(char* target, size_t capacity, const char* source) {
    if (capacity == 0) return;
    if (source == nullptr) {
        target[0] = '\0';
        return;
    }
    std::snprintf(target, capacity, "%s", source);
}

int open_decoder(NeriFfDecoder* decoder, const char* format_hint, bool drc_off) {
    auto* buffer = static_cast<unsigned char*>(api.av_malloc(kIoBufferSize));
    if (buffer == nullptr) return fail(decoder, "out of memory", AVERROR(ENOMEM));
    decoder->avio = api.avio_alloc_context(buffer, kIoBufferSize, 0, decoder, &io_read, nullptr,
                                           decoder->io.seek != nullptr ? &io_seek : nullptr);
    if (decoder->avio == nullptr) {
        api.av_free(buffer);
        return fail(decoder, "out of memory", AVERROR(ENOMEM));
    }
    decoder->format = api.avformat_alloc_context();
    if (decoder->format == nullptr) return fail(decoder, "out of memory", AVERROR(ENOMEM));
    decoder->format->pb = decoder->avio;
    decoder->format->flags |= AVFMT_FLAG_CUSTOM_IO;
    decoder->format->interrupt_callback.callback = &io_interrupted;
    decoder->format->interrupt_callback.opaque = decoder;
    // 纯音频不需要默认 5 MB 的探测量；压低它能缩短远程流的起播时间
    decoder->format->probesize = 512 * 1024;
    decoder->format->max_analyze_duration = AV_TIME_BASE / 2;

    const AVInputFormat* input_format = nullptr;
    if (format_hint != nullptr && format_hint[0] != '\0') input_format = api.av_find_input_format(format_hint);
    int result = api.avformat_open_input(&decoder->format, nullptr, input_format, nullptr);
    if (result < 0) {
        decoder->format = nullptr;  // 打开失败时 FFmpeg 已经释放了上下文
        return fail(decoder, "could not open the input", result);
    }
    result = api.avformat_find_stream_info(decoder->format, nullptr);
    if (result < 0) return fail(decoder, "could not read stream information", result);

    const AVCodec* codec = nullptr;
    result = api.av_find_best_stream(decoder->format, AVMEDIA_TYPE_AUDIO, -1, -1, &codec, 0);
    if (result < 0) {
        return fail(decoder,
                    result == AVERROR_DECODER_NOT_FOUND ? "no decoder for the audio stream" : "no audio stream",
                    result);
    }
    decoder->stream_index = result;
    AVStream* stream = decoder->format->streams[result];
    decoder->time_base = stream->time_base;
    decoder->stream_start = stream->start_time != AV_NOPTS_VALUE ? stream->start_time : 0;

    decoder->codec = api.avcodec_alloc_context3(codec);
    if (decoder->codec == nullptr) return fail(decoder, "out of memory", AVERROR(ENOMEM));
    result = api.avcodec_parameters_to_context(decoder->codec, stream->codecpar);
    if (result < 0) return fail(decoder, "could not configure the decoder", result);
    decoder->codec->pkt_timebase = stream->time_base;

    const AVCodecID codec_id = stream->codecpar->codec_id;
    const bool dolby = codec_id == AV_CODEC_ID_AC3 || codec_id == AV_CODEC_ID_EAC3;
    const bool wants_decoder_downmix = dolby && stream->codecpar->ch_layout.nb_channels > decoder->max_output_channels;
    AVDictionary* options = nullptr;
    // AC-3/E-AC-3 由解码器自己下混：它按码流里的中置/环绕混音电平与 Lo/Ro、Lt/Rt 偏好计算
    if (wants_decoder_downmix) {
        api.av_dict_set(&options, "downmix", decoder->max_output_channels == 1 ? "mono" : "stereo", 0);
    }
    if (dolby && drc_off) api.av_dict_set(&options, "drc_scale", "0", 0);
    result = api.avcodec_open2(decoder->codec, codec, &options);
    const bool decoder_downmix = wants_decoder_downmix && api.av_dict_get(options, "downmix", nullptr, 0) == nullptr;
    api.av_dict_free(&options);
    if (result < 0) return fail(decoder, "could not open the decoder", result);

    decoder->packet = api.av_packet_alloc();
    decoder->frame = api.av_frame_alloc();
    if (decoder->packet == nullptr || decoder->frame == nullptr) return fail(decoder, "out of memory", AVERROR(ENOMEM));

    // 打开时就解出第一帧：不可解的流在这里失败，调用方能立即换后端，而不是起播后才发现
    result = receive_frame(decoder);
    if (result == 0) return fail(decoder, "the stream contains no decodable audio", AVERROR_EOF);
    if (result < 0) return result;
    decoder->frame_ready = true;
    decoder->sample_rate = decoder->frame->sample_rate > 0 ? decoder->frame->sample_rate : decoder->codec->sample_rate;
    configure_channels(decoder);

    NeriFfStreamInfo& info = decoder->info;
    info.sample_rate = decoder->sample_rate;
    info.source_channels = stream->codecpar->ch_layout.nb_channels;
    info.output_channels = decoder->output_channels;
    info.bits_per_sample = stream->codecpar->bits_per_raw_sample > 0 ? stream->codecpar->bits_per_raw_sample
                                                                      : stream->codecpar->bits_per_coded_sample;
    info.bit_rate = stream->codecpar->bit_rate > 0 ? stream->codecpar->bit_rate : decoder->format->bit_rate;
    if (decoder->format->duration != AV_NOPTS_VALUE && decoder->format->duration > 0) {
        info.duration_us = decoder->format->duration;
    } else if (stream->duration != AV_NOPTS_VALUE && stream->duration > 0) {
        info.duration_us = api.av_rescale_q(stream->duration, stream->time_base, AVRational{1, 1000000});
    } else {
        info.duration_us = -1;
    }
    info.initial_padding = stream->codecpar->initial_padding;
    info.trailing_padding = stream->codecpar->trailing_padding;
    info.seek_preroll = stream->codecpar->seek_preroll;
    info.decoder_downmix = decoder_downmix ? 1 : 0;
    info.matrix_downmix = decoder->use_matrix ? 1 : 0;
    copy_cstr(info.codec, sizeof(info.codec), api.avcodec_get_name(codec_id));
    copy_cstr(info.container, sizeof(info.container), decoder->format->iformat ? decoder->format->iformat->name : "");
    return 0;
}

}  // namespace

extern "C" {

void neri_ff_set_log_sink(void (*sink)(int32_t level, const char* line)) {
    log_sink.store(sink, std::memory_order_release);
}

int32_t neri_ff_load(const char* directory) {
    std::lock_guard<std::mutex> guard(load_mutex);
    if (load_state.load(std::memory_order_acquire) == 1) return 0;
    load_error[0] = '\0';
    if (directory == nullptr || directory[0] == '\0') {
        std::snprintf(load_error, sizeof(load_error), "no FFmpeg directory given");
        load_state.store(-1, std::memory_order_release);
        return -1;
    }

    LibHandle loaded[3] = {};
    for (int index = 0; index < 3; ++index) {
        loaded[index] = open_library(directory, kLibraryNames[index], load_error, sizeof(load_error));
        if (loaded[index] == nullptr) {
            for (int opened = 0; opened < index; ++opened) close_library(loaded[opened]);
            load_state.store(-1, std::memory_order_release);
            return -1;
        }
    }

    FfApi candidate{};
    bool resolved = true;
#define NERI_FF_BIND(name)                                                                       \
    candidate.name = reinterpret_cast<decltype(candidate.name)>(find_symbol(handle, #name));     \
    if (candidate.name == nullptr && resolved) {                                                 \
        std::snprintf(load_error, sizeof(load_error), "%s does not export %s", library, #name); \
        resolved = false;                                                                        \
    }
    {
        LibHandle handle = loaded[0];
        const char* library = kLibraryNames[0];
        NERI_FF_AVUTIL(NERI_FF_BIND)
    }
    {
        LibHandle handle = loaded[1];
        const char* library = kLibraryNames[1];
        NERI_FF_AVCODEC(NERI_FF_BIND)
    }
    {
        LibHandle handle = loaded[2];
        const char* library = kLibraryNames[2];
        NERI_FF_AVFORMAT(NERI_FF_BIND)
    }
#undef NERI_FF_BIND

    if (resolved) {
        const unsigned avutil = candidate.avutil_version();
        const unsigned avcodec = candidate.avcodec_version();
        const unsigned avformat = candidate.avformat_version();
        if (!version_compatible(avutil, LIBAVUTIL_VERSION_MAJOR, LIBAVUTIL_VERSION_MINOR) ||
            !version_compatible(avcodec, LIBAVCODEC_VERSION_MAJOR, LIBAVCODEC_VERSION_MINOR) ||
            !version_compatible(avformat, LIBAVFORMAT_VERSION_MAJOR, LIBAVFORMAT_VERSION_MINOR)) {
            std::snprintf(load_error, sizeof(load_error),
                          "incompatible FFmpeg: avutil %u.%u, avcodec %u.%u, avformat %u.%u; "
                          "needs avutil %d.%d+, avcodec %d.%d+, avformat %d.%d+",
                          AV_VERSION_MAJOR(avutil), AV_VERSION_MINOR(avutil), AV_VERSION_MAJOR(avcodec),
                          AV_VERSION_MINOR(avcodec), AV_VERSION_MAJOR(avformat), AV_VERSION_MINOR(avformat),
                          LIBAVUTIL_VERSION_MAJOR, LIBAVUTIL_VERSION_MINOR, LIBAVCODEC_VERSION_MAJOR,
                          LIBAVCODEC_VERSION_MINOR, LIBAVFORMAT_VERSION_MAJOR, LIBAVFORMAT_VERSION_MINOR);
            resolved = false;
        }
    }
    if (!resolved) {
        for (LibHandle handle : loaded) close_library(handle);
        load_state.store(-1, std::memory_order_release);
        return -1;
    }

    api = candidate;
    for (int index = 0; index < 3; ++index) libraries[index] = loaded[index];
    api.av_log_set_level(AV_LOG_WARNING);
    api.av_log_set_callback(&forward_log);
    load_state.store(1, std::memory_order_release);
    return 0;
}

const char* neri_ff_load_error(void) {
    return load_error;
}

void neri_ff_versions(uint32_t* avutil, uint32_t* avcodec, uint32_t* avformat) {
    const bool loaded = load_state.load(std::memory_order_acquire) == 1;
    if (avutil != nullptr) *avutil = loaded ? api.avutil_version() : 0;
    if (avcodec != nullptr) *avcodec = loaded ? api.avcodec_version() : 0;
    if (avformat != nullptr) *avformat = loaded ? api.avformat_version() : 0;
}

int32_t neri_ff_has_decoder(const char* name) {
    if (load_state.load(std::memory_order_acquire) != 1 || name == nullptr) return 0;
    return api.avcodec_find_decoder_by_name(name) != nullptr ? 1 : 0;
}

int32_t neri_ff_has_demuxer(const char* name) {
    if (load_state.load(std::memory_order_acquire) != 1 || name == nullptr) return 0;
    return api.av_find_input_format(name) != nullptr ? 1 : 0;
}

int32_t neri_ff_open(const NeriFfIo* io, const char* format_hint, int32_t max_output_channels, int32_t drc_off,
                     NeriFfDecoder** out, char* error, int32_t error_len) {
    if (out != nullptr) *out = nullptr;
    auto report = [&](const char* message) {
        if (error != nullptr && error_len > 0) copy_cstr(error, static_cast<size_t>(error_len), message);
    };
    if (load_state.load(std::memory_order_acquire) != 1) {
        report("FFmpeg is not loaded");
        return -1;
    }
    if (io == nullptr || io->read == nullptr || out == nullptr) {
        report("invalid arguments");
        return -1;
    }
    auto* decoder = new (std::nothrow) NeriFfDecoder();
    if (decoder == nullptr) {
        report("out of memory");
        return -1;
    }
    decoder->io = *io;
    decoder->max_output_channels = max_output_channels > 0 && max_output_channels <= 2 ? max_output_channels : 2;
    const int result = open_decoder(decoder, format_hint, drc_off != 0);
    if (result < 0) {
        report(decoder->error[0] != '\0' ? decoder->error : "could not open the stream");
        destroy(decoder);
        return result;
    }
    *out = decoder;
    return 0;
}

int32_t neri_ff_info(const NeriFfDecoder* decoder, NeriFfStreamInfo* out) {
    if (decoder == nullptr || out == nullptr) return -1;
    *out = decoder->info;
    return 0;
}

int32_t neri_ff_read(NeriFfDecoder* decoder, float* out, int32_t max_frames) {
    if (decoder == nullptr || out == nullptr || max_frames <= 0) return -1;
    int32_t written = 0;
    while (written < max_frames) {
        if (!decoder->frame_ready) {
            const int result = next_ready_frame(decoder);
            if (result == 0) break;
            if (result < 0) {
                // 先把已经解出的部分交出去，错误留给下一次调用报告
                if (written > 0) break;
                return result;
            }
            decoder->frame_ready = true;
        }
        const AVFrame* frame = decoder->frame;
        const int available = frame->nb_samples - decoder->frame_offset;
        const int count = available < max_frames - written ? available : max_frames - written;
        const int outputs = decoder->output_channels;
        for (int sample = 0; sample < count; ++sample) {
            const int index = decoder->frame_offset + sample;
            float* target = out + static_cast<size_t>(written + sample) * outputs;
            if (decoder->use_matrix) {
                for (int output = 0; output < outputs; ++output) {
                    float mixed = 0.0f;
                    for (int channel = 0; channel < decoder->frame_channels && channel < kMaxChannels; ++channel) {
                        mixed += decoder->matrix[output][channel] * sample_at(frame, decoder->frame_channels, channel, index);
                    }
                    target[output] = mixed;
                }
            } else {
                for (int channel = 0; channel < outputs; ++channel) {
                    target[channel] = sample_at(frame, decoder->frame_channels, channel, index);
                }
            }
        }
        decoder->frame_offset += count;
        written += count;
        if (decoder->frame_offset >= frame->nb_samples) {
            api.av_frame_unref(decoder->frame);
            decoder->frame_ready = false;
        }
    }
    return written;
}

int32_t neri_ff_seek(NeriFfDecoder* decoder, int64_t target_us) {
    if (decoder == nullptr || target_us < 0) return -1;
    const int64_t preroll_us = decoder->info.seek_preroll > 0 && decoder->sample_rate > 0
                                   ? static_cast<int64_t>(decoder->info.seek_preroll) * 1000000 / decoder->sample_rate
                                   : 0;
    const int64_t seek_us = target_us > preroll_us ? target_us - preroll_us : 0;
    const int64_t timestamp = api.av_rescale_q(seek_us, AVRational{1, 1000000}, decoder->time_base) + decoder->stream_start;
    int result = api.av_seek_frame(decoder->format, decoder->stream_index, timestamp, AVSEEK_FLAG_BACKWARD);
    if (result < 0) {
        result = api.av_seek_frame(decoder->format, decoder->stream_index, timestamp, AVSEEK_FLAG_BACKWARD | AVSEEK_FLAG_ANY);
    }
    if (result < 0) return fail(decoder, "seeking failed", result);
    api.avcodec_flush_buffers(decoder->codec);
    api.av_frame_unref(decoder->frame);
    decoder->frame_ready = false;
    decoder->frame_offset = 0;
    decoder->input_finished = false;
    decoder->consecutive_errors = 0;
    decoder->discard_before = target_us * decoder->sample_rate / 1000000;
    decoder->error[0] = '\0';
    return 0;
}

const char* neri_ff_decoder_error(const NeriFfDecoder* decoder) {
    return decoder != nullptr ? decoder->error : "";
}

void neri_ff_close(NeriFfDecoder* decoder) {
    destroy(decoder);
}

}  // extern "C"
