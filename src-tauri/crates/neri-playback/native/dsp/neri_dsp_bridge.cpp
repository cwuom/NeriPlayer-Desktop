#include "dsp/neri_dsp_engine.h"

#include <array>
#include <new>
#include <memory>

using namespace neri::dsp;

namespace {

struct DesktopEngine {
    std::array<Engine, 3> modes;
    int selected = 1;
};

double magnitudeDb(const BiquadCoefficients& c, double frequency) {
    const double w = 2.0 * kPi * frequency / 48000.0;
    const double cw = std::cos(w);
    const double c2w = std::cos(2.0 * w);
    const double numerator = c.b0 * c.b0 + c.b1 * c.b1 + c.b2 * c.b2
        + 2.0 * (c.b0 * c.b1 + c.b1 * c.b2) * cw + 2.0 * c.b0 * c.b2 * c2w;
    const double denominator = 1.0 + c.a1 * c.a1 + c.a2 * c.a2
        + 2.0 * (c.a1 + c.a1 * c.a2) * cw + 2.0 * c.a2 * c2w;
    return numerator > 0.0 && denominator > 0.0 ? 10.0 * std::log10(numerator / denominator) : -120.0;
}

} // namespace

extern "C" {

struct NeriDesktopDspStats {
    int64_t frames;
    int64_t nanos;
    float limiterReductionDb;
    float compressorReductionDb;
    int32_t idle;
    int32_t sampleRate;
};

void* neri_desktop_dsp_create(int32_t sampleRate, int32_t channels, const float* neutral) noexcept {
    try {
        auto desktop = std::make_unique<DesktopEngine>();
        for (int mode = 0; mode < 3; ++mode) {
            std::array<float, kParamCount> params;
            std::copy(neutral, neutral + kParamCount, params.begin());
            params[kQualityMode] = static_cast<float>(mode);
            if (!desktop->modes[mode].setParams(params.data(), kParamCount)
                || !desktop->modes[mode].configure(sampleRate, channels, kEncodingPcmFloat)) {
                return nullptr;
            }
        }
        return desktop.release();
    } catch (...) {
        return nullptr;
    }
}

void neri_desktop_dsp_destroy(void* pointer) noexcept {
    delete static_cast<DesktopEngine*>(pointer);
}

int32_t neri_desktop_dsp_set_params(void* pointer, const float* params, int32_t count) noexcept {
    if (!pointer || !params || count != kParamCount) return 0;
    for (int i = 0; i < count; ++i) if (!std::isfinite(params[i])) return 0;
    auto& desktop = *static_cast<DesktopEngine*>(pointer);
    const int mode = params[kMasterEnabled] > 0.5f
        ? static_cast<int>(std::lround(clampf(params[kQualityMode], 0.0f, 2.0f))) : desktop.selected;
    std::array<float, kParamCount> packet;
    std::copy(params, params + kParamCount, packet.begin());
    packet[kQualityMode] = static_cast<float>(mode);
    if (mode != desktop.selected) {
        desktop.selected = mode;
        // 预建模式保留容量，清掉上次使用的尾音后从直通重新淡入
        std::array<float, kParamCount> disabled;
        std::copy(packet.begin(), packet.end(), disabled.begin());
        disabled[kMasterEnabled] = 0.0f;
        desktop.modes[mode].setParams(disabled.data(), count);
        desktop.modes[mode].resetToIdle();
    }
    return desktop.modes[mode].setParams(packet.data(), count) ? 1 : 0;
}

int32_t neri_desktop_dsp_process(void* pointer, float* samples, size_t count) noexcept {
    if (!pointer) return -1;
    auto& desktop = *static_cast<DesktopEngine*>(pointer);
    auto* bytes = reinterpret_cast<uint8_t*>(samples);
    return desktop.modes[desktop.selected].process(bytes, bytes, count * sizeof(float));
}

void neri_desktop_dsp_stats(void* pointer, NeriDesktopDspStats* output) noexcept {
    if (!pointer || !output) return;
    auto& desktop = *static_cast<DesktopEngine*>(pointer);
    auto& engine = desktop.modes[desktop.selected];
    const auto stats = engine.takeStats();
    *output = {stats.processedFrames, stats.processingNanos, stats.maxLimiterReductionDb,
        stats.compressorReductionDb, engine.idle() ? 1 : 0, stats.sampleRate};
}

float neri_desktop_dsp_headroom(const float* p) noexcept {
    std::array<BiquadCoefficients, kGraphicBandCount + kParametricBandCount + 5> filters;
    size_t count = 0;
    if (p[kGraphicEqEnabled] > 0.5f) {
        for (int band = 0; band < kGraphicBandCount; ++band) {
            filters[count++] = designBiquad(FilterType::Peak, 48000.0,
                kGraphicBandFrequenciesHz[band], p[kGraphicEqGain0 + band], 1.41);
        }
    }
    if (p[kParametricEqEnabled] > 0.5f) {
        for (int band = 0; band < kParametricBandCount; ++band) {
            const int base = kParametricBand0 + band * kParametricBandStride;
            if (p[base + kBandEnabled] > 0.5f) {
                filters[count++] = designBiquad(filterTypeFromParam(p[base + kBandType]), 48000.0,
                    p[base + kBandFrequencyHz], p[base + kBandGainDb], p[base + kBandQ]);
            }
        }
    }
    if (p[kBassGainDb] != 0.0f) {
        // 增强限量承担低频峰值，前级只保留三成正搁架余量，避免把中高频一起压低
        constexpr float kBassShelfHeadroomShare = 0.3f;
        const float gainDb = p[kBassGainDb] > 0.0f ? p[kBassGainDb] * kBassShelfHeadroomShare : p[kBassGainDb];
        filters[count++] = designBiquad(FilterType::LowShelf, 48000.0, p[kBassFrequencyHz], gainDb, 0.707);
        const float subsonicHz = bassSubsonicFrequencyHz(p[kBassGainDb], p[kBassFrequencyHz]);
        if (subsonicHz > 0.0f) {
            filters[count++] = designBiquad(FilterType::HighPass, 48000.0, subsonicHz, 0.0, 0.707);
        }
    }
    if (p[kTrebleGainDb] != 0.0f) filters[count++] = designBiquad(FilterType::HighShelf, 48000.0, p[kTrebleFrequencyHz], p[kTrebleGainDb], 0.707);
    if (p[kLoudnessBassDb] > 0.0f) filters[count++] = designBiquad(FilterType::LowShelf, 48000.0, kLoudnessBassHz, p[kLoudnessBassDb], 0.707);
    if (p[kLoudnessTrebleDb] > 0.0f) filters[count++] = designBiquad(FilterType::HighShelf, 48000.0, kLoudnessTrebleHz, p[kLoudnessTrebleDb], 0.707);
    double maxBoost = 0.0;
    for (int index = 0; index < 160; ++index) {
        const double frequency = std::exp(std::log(20.0) + (std::log(20000.0) - std::log(20.0)) * index / 159.0);
        double gain = 0.0;
        for (size_t i = 0; i < count; ++i) gain += magnitudeDb(filters[i], frequency);
        maxBoost = std::max(maxBoost, gain);
    }
    const double enhancement = 2.5 * p[kVirtualBassAmount] + 1.5 * p[kExciterAmount]
        + p[kWarmthAmount] + 1.5 * p[kVocalClarityAmount];
    return static_cast<float>(std::clamp(0.85 * maxBoost + enhancement, 0.0, 18.0));
}

} // extern "C"
