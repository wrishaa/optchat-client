# source me: cross env for armv7 tizen (tv QA55Q60C, 32-bit soft-float abi). needs tizen studio (TIZEN_STUDIO, default
# ~/tizen-studio) with the tv 9.0 rootstrap and the arm gcc 14.2 toolchain. run from the repo root.
T=${TIZEN_STUDIO:-$HOME/tizen-studio}
H=$(pwd)/shells/tizen
export TZ_ROOT=$T/platforms/tizen-9.0/tizen/rootstraps/tizen-9.0-device.core
export TCC=$H/toolchain/tcc
export CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABI_LINKER=$TCC
export CC_armv7_unknown_linux_gnueabi=$TCC CXX_armv7_unknown_linux_gnueabi=$TCC
export AR_armv7_unknown_linux_gnueabi=$T/tools/llvm-10/bin/llvm-ar
export PKG_CONFIG_ALLOW_CROSS=1 PKG_CONFIG_SYSROOT_DIR=$TZ_ROOT
export PKG_CONFIG_LIBDIR=$TZ_ROOT/usr/lib/pkgconfig
# link stub for libxkbcommon (not in the rootstrap; the real one is on the tv), built by build.sh
STUBS=$(pwd)/target/tizen-stubs
export CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABI_RUSTFLAGS="-L $STUBS -C link-arg=-Wl,--allow-shlib-undefined"
export RUST_FONTCONFIG_DLOPEN=1
export OPENSSL_LIB_DIR=$TZ_ROOT/usr/lib OPENSSL_INCLUDE_DIR=$TZ_ROOT/usr/include
