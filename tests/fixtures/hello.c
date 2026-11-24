// Fixture for the kuna driver tests: a tiny static x86-64 Linux ELF, only ever analysed, never run.
// Rebuild (clang + rustup's rust-lld, any host):
//   clang -target x86_64-unknown-linux-gnu -ffreestanding -nostdlib -fno-stack-protector -fno-pic \
//     -fno-asynchronous-unwind-tables -O1 -c hello.c -o hello.o
//   rust-lld -flavor gnu -static -nostdlib -e _start -o hello.elf hello.o

__attribute__((noinline)) int square(int x) { return x * x; }

__attribute__((noinline)) int add(int a, int b) { return a + b; }

int main(void) { return add(square(3), 4); }

void _start(void) {
    int status = main();
    __asm__ volatile("syscall" : : "a"(60), "D"(status));
    for (;;) {
    }
}
