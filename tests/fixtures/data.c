// Fixture for the data views: a tiny static x86-64 Linux ELF, only ever analysed, never run. It has a string, a TEA
// delta immediate (crypto) and a 44-byte gzip file (blobs), each referenced by one function.
// Rebuild (clang + rustup's rust-lld, any host):
//   clang -target x86_64-unknown-linux-gnu -ffreestanding -nostdlib -fno-stack-protector -fno-pic \
//     -fno-asynchronous-unwind-tables -O1 -c data.c -o data.o
//   rust-lld -flavor gnu -static -nostdlib -e _start -o data.elf data.o

static const char greeting[] = "hello from bonito";

// printf 'bonito fixture blob, bonito fixture blob' | gzip -9n
static const unsigned char blob[] = {
    0x1f, 0x8b, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x03, 0x4b, 0xca, 0xcf, 0xcb, 0x2c,
    0xc9, 0x57, 0x48, 0xcb, 0xac, 0x28, 0x29, 0x2d, 0x4a, 0x55, 0x48, 0xca, 0xc9, 0x4f, 0xd2,
    0x51, 0x48, 0xc2, 0x14, 0x04, 0x00, 0x62, 0x01, 0xf1, 0x7a, 0x28, 0x00, 0x00, 0x00};

__attribute__((noinline)) const char *say(void) { return greeting; }

__attribute__((noinline)) const unsigned char *packed(void) { return blob; }

__attribute__((noinline)) unsigned tea_step(unsigned sum) { return sum + 0x9E3779B9u; }

void _start(void) {
    unsigned status = tea_step((unsigned)say()[0] + packed()[0]);
    __asm__ volatile("syscall" : : "a"(60), "D"(status));
    for (;;) {
    }
}
