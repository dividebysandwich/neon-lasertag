# Trunk's post_build hook (Trunk.toml), in the staging directory of a build.
set -e
# Gzipped copies of the wasm and scripts beside them, for nginx's gzip_static (docs/web.md),
# and the gzipped wasm's size for the loading screen (web/loader.js).
(
  cd "$TRUNK_STAGING_DIR"
  gzip -9 -k -f *.wasm
  printf 'window.LT_WASM_GZ = %s;\n' "$(cat *.wasm.gz | wc -c)" > version.js
  gzip -9 -k -f *.js
)
# The WebGL2 build (without the webgpu feature) in gl/, for browsers without WebGPU: this page
# marked data-gl, with gl/'s scripts and the assets from the WebGPU build's directory, so they
# aren't there twice.
[ -n "$LT_GL_BUILD" ] && exit 0
sed -e 's|<html lang="en">|<html lang="en" data-gl>|' \
    -e 's|<head>|<head>\n  <base href="../">|' \
    -e 's| data-cargo-features="webgpu"||' \
    -e 's|<script src="version.js">|<script src="gl/version.js">|' \
    -e '/rel="copy-dir"/d' \
    web/index.html > web/index.gl.html
release=
[ "$TRUNK_PROFILE" = release ] && release=--release=true
LT_GL_BUILD=1 trunk build $release --dist "$TRUNK_STAGING_DIR/gl" --public-url ./gl/ \
  web/index.gl.html
