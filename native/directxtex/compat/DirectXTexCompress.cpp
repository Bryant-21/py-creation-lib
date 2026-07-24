#include "../external/DirectXTex/DirectXTex/DirectXTexP.h"

#ifdef _OPENMP
#undef null // ponytail: vendored sal.h #defines null empty, which breaks omp.h's default-arg macros on GCC
#endif

#include "../external/DirectXTex/DirectXTex/DirectXTexCompress.cpp"
