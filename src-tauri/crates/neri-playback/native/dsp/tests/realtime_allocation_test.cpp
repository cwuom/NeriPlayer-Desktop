#define main upstream_regressions_main
#include "neri_dsp_engine_test.cpp"
#undef main

#include <new>

namespace {
thread_local bool trackingAllocations = false;
thread_local size_t allocationCount = 0;
}

void* operator new(size_t bytes) {
    if (trackingAllocations) ++allocationCount;
    if (void* memory = std::malloc(bytes == 0 ? 1 : bytes)) return memory;
    throw std::bad_alloc();
}

void* operator new[](size_t bytes) { return ::operator new(bytes); }
void operator delete(void* memory) noexcept { std::free(memory); }
void operator delete[](void* memory) noexcept { std::free(memory); }
void operator delete(void* memory, size_t) noexcept { std::free(memory); }
void operator delete[](void* memory, size_t) noexcept { std::free(memory); }

extern "C" {
void* neri_desktop_dsp_create(int32_t rate, int32_t channels, const float* neutral) noexcept;
void neri_desktop_dsp_destroy(void* pointer) noexcept;
int32_t neri_desktop_dsp_set_params(void* pointer, const float* params, int32_t count) noexcept;
int32_t neri_desktop_dsp_process(void* pointer, float* samples, size_t count) noexcept;
}

int main() {
    auto params = neutralParams();
    params[kMasterEnabled] = 0.0f;
    void* desktop = neri_desktop_dsp_create(kSampleRate, 2, params.data());
    if (!desktop) return 1;
    std::array<float, 960> samples{};
    bool valid = true;
    trackingAllocations = true;
    for (int iteration = 0; iteration < 300; ++iteration) {
        params[kQualityMode] = static_cast<float>(iteration % 3);
        params[kMasterEnabled] = iteration % 7 == 0 ? 0.0f : 1.0f;
        params[kGraphicEqEnabled] = 1.0f;
        params[kGraphicEqGain0 + 5] = static_cast<float>(iteration % 12);
        params[kBassGainDb] = iteration % 2 == 0 ? 9.0f : -6.0f;
        params[kBassFrequencyHz] = 40.0f + static_cast<float>(iteration % 211);
        params[kTrebleGainDb] = iteration % 3 == 0 ? -6.0f : 9.0f;
        params[kTrebleFrequencyHz] = 1000.0f + static_cast<float>((iteration * 137) % 15001);
        params[kVirtualBassAmount] = iteration % 5 == 0 ? 0.0f : 0.6f;
        params[kVirtualBassFrequencyHz] = 50.0f + static_cast<float>(iteration % 151);
        params[kWarmthAmount] = 0.8f;
        params[kSurroundAmount] = 0.8f;
        params[kReverbAmount] = 0.8f;
        params[kReverbPreDelayMs] = static_cast<float>(iteration % 100);
        params[kSpeakerEnabled] = 1.0f;
        params[kSpeakerBassHarmonics] = 0.5f;
        params[kSpeakerHighPassHz] = 40.0f + static_cast<float>((iteration * 17) % 360);
        params[kSpeakerLoudness] = static_cast<float>(iteration % 101) / 100.0f;
        params[kCompressorEnabled] = 1.0f;
        params[kCompressorMakeupDb] = static_cast<float>(iteration % 19);
        params[kLimiterEnabled] = 1.0f;
        for (size_t frame = 0; frame < samples.size() / 2; ++frame) {
            const double phase = 2.0 * kPi * static_cast<double>(frame) / kSampleRate;
            const float sample = static_cast<float>(0.2 * std::sin(55.0 * phase)
                + 0.1 * std::sin(1000.0 * phase) + 0.1 * std::sin(12000.0 * phase));
            samples[frame * 2] = sample;
            samples[frame * 2 + 1] = sample;
        }
        valid = neri_desktop_dsp_set_params(desktop, params.data(), kParamCount) != 0 && valid;
        valid = neri_desktop_dsp_process(desktop, samples.data(), samples.size()) >= 0 && valid;
        for (float sample : samples) valid = std::isfinite(sample) && valid;
    }
    trackingAllocations = false;
    neri_desktop_dsp_destroy(desktop);
    std::printf("desktop realtime DSP: %zu allocations, valid=%d, 300 updates across all quality modes\n",
        allocationCount, valid ? 1 : 0);
    return valid && allocationCount == 0 ? 0 : 1;
}
