#define main android_dsp_tests_main
#include "neri_dsp_engine_test.cpp"
#undef main

extern "C" int neri_desktop_dsp_run_host_tests() {
    return android_dsp_tests_main();
}
