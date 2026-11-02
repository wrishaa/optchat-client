# source me: cross env for armv7 tizen (tv QA55Q60C, 32-bit soft-float abi)
T=$HOME/tizen-studio
export TZ_ROOT=$T/platforms/tizen-9.0/tizen/rootstraps/tizen-9.0-device.core
export TCC=$HOME/optchat-scratch/native/tv/bin/tcc
export CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABI_LINKER=$TCC
export CC_armv7_unknown_linux_gnueabi=$TCC CXX_armv7_unknown_linux_gnueabi=$TCC
export AR_armv7_unknown_linux_gnueabi=$T/tools/llvm-10/bin/llvm-ar
export PKG_CONFIG_ALLOW_CROSS=1 PKG_CONFIG_SYSROOT_DIR=$TZ_ROOT
export PKG_CONFIG_LIBDIR=$TZ_ROOT/usr/lib/pkgconfig:$HOME/optchat-scratch/native/tv/stubs/pc
export CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABI_RUSTFLAGS="-L $HOME/optchat-scratch/native/tv/stubs/lib -C link-arg=-Wl,--allow-shlib-undefined"
export RUST_FONTCONFIG_DLOPEN=1
