/* cxsym-inject — load a .so into a RUNNING Linux x86_64 process.
 *
 * Purpose-built for CXSFM (CarX Street Framework Mod). Technique inspired
 * by classic ptrace injectors, but with one critical difference:
 *
 *   Classic injectors resolve the target's mmap/dlopen addresses by
 *   offset from their OWN libc (remote_base + (local_sym - local_base)).
 *   That silently assumes injector and target use the SAME libc build.
 *   On NixOS (injector = nix glibc) vs Steam Runtime (game = older glibc)
 *   the offsets differ, the remote call jumps into garbage, and the game
 *   dies. This injector instead walks the TARGET's own link_map
 *   (via DT_DEBUG) and parses the TARGET libc's symbol tables, so it is
 *   correct regardless of which libc either side uses.
 *
 * Steps: PTRACE_ATTACH → find exe base → PT_DYNAMIC → DT_DEBUG → r_map →
 * link_map walk to libc → parse its DT_SYMTAB (SYSV hash) for mmap +
 * __libc_dlopen_mode → remote mmap(RW) → poke .so path → remote dlopen →
 * report dlerror on failure → PTRACE_DETACH.
 *
 * Usage: cxsym-inject <pid> /absolute/path/to.so
 */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/ptrace.h>
#include <sys/uio.h>
#include <sys/user.h>
#include <sys/wait.h>
#include <unistd.h>
#include <stdint.h>
#include <limits.h>
#include <elf.h>

#define DL_MODE (RTLD_NOW | RTLD_LOCAL)

static void die(const char *msg)
{
	perror(msg);
	exit(1);
}

static int wait_stopped(pid_t pid)
{
	int status;
	for (;;) {
		if (waitpid(pid, &status, 0) < 0)
			die("waitpid");
		if (WIFEXITED(status) || WIFSIGNALED(status)) {
			fprintf(stderr, "Target process exited while attaching.\n");
			return -1;
		}
		if (WIFSTOPPED(status))
			return status;
	}
}

/* Read `len` bytes of the TARGET's memory. Returns 0 on success. */
static int target_read(pid_t pid, unsigned long addr, void *buf, size_t len)
{
	struct iovec local = { buf, len };
	struct iovec remote = { (void *)addr, len };
	ssize_t n = process_vm_readv(pid, &local, 1, &remote, 1, 0);
	if (n < 0 || (size_t)n != len) {
		fprintf(stderr, "target_read failed at 0x%lx (%zu bytes).\n",
			addr, len);
		return -1;
	}
	return 0;
}

/* Read a NUL-terminated string from the target (bounded). */
static int target_read_str(pid_t pid, unsigned long addr, char *out,
			   size_t out_sz)
{
	size_t i = 0;
	while (i + 1 < out_sz) {
		unsigned char c;
		if (target_read(pid, addr + i, &c, 1) < 0)
			return -1;
		out[i] = (char)c;
		if (!c)
			return 0;
		i++;
	}
	out[out_sz - 1] = '\0';
	return 0;
}

/* Start address of the target's first mapping = executable base. */
static unsigned long target_exe_base(pid_t pid)
{
	char maps[64];
	snprintf(maps, sizeof(maps), "/proc/%d/maps", (int)pid);
	FILE *f = fopen(maps, "r");
	if (!f)
		die("fopen target maps");
	char line[512];
	unsigned long base = 0;
	if (fgets(line, sizeof(line), f)) {
		if (sscanf(line, "%lx-%*x", &base) != 1)
			base = 0;
	}
	fclose(f);
	return base;
}

static int read_u64(pid_t pid, unsigned long addr, uint64_t *out)
{
	return target_read(pid, addr, out, sizeof(*out));
}

/* Find a dynamic tag in the target's DYNAMIC array. */
static int find_dyn(pid_t pid, unsigned long dyn_addr, int64_t tag,
		    uint64_t *out_val)
{
	for (int i = 0; i < 256; i++) {
		Elf64_Dyn dyn;
		if (target_read(pid, dyn_addr + (unsigned long)i * sizeof(dyn),
				 &dyn, sizeof(dyn)) < 0)
			return -1;
		if (dyn.d_tag == DT_NULL)
			return -1;
		if (dyn.d_tag == tag) {
			*out_val = dyn.d_un.d_ptr;
			return 0;
		}
	}
	return -1;
}

/* SYSV ELF hash (matches DT_HASH tables). */
static unsigned long elf_hash(const char *name)
{
	unsigned long h = 0, g;
	while (*name) {
		h = (h << 4) + (unsigned char)*name++;
		g = h & 0xf0000000UL;
		if (g)
			h ^= g >> 24;
		h &= ~g;
	}
	return h;
}

/*
 * Resolve `symname` inside the target's libc:
 * exe base → PT_DYNAMIC → DT_DEBUG → r_map → link_map walk to the entry
 * whose filename contains "libc.so" → parse its DT_SYMTAB via DT_HASH.
 * Returns the symbol's RUNTIME address, or 0 on failure.
 */
static unsigned long resolve_target_libc_sym(pid_t pid, const char *symname)
{
	/* --- executable base + PT_DYNAMIC (PIE-aware via load bias) --- */
	unsigned long exe_map = target_exe_base(pid);
	if (!exe_map) {
		fprintf(stderr, "No mappings in target.\n");
		return 0;
	}
	unsigned char eident[EI_NIDENT];
	if (target_read(pid, exe_map, eident, sizeof(eident)) < 0)
		return 0;
	if (memcmp(eident, ELFMAG, SELFMAG) != 0 ||
	    eident[EI_CLASS] != ELFCLASS64) {
		fprintf(stderr, "Target executable is not 64-bit ELF.\n");
		return 0;
	}
	uint64_t e_phoff, e_phnum;
	uint16_t phnum16;
	if (target_read(pid, exe_map + 0x20, &e_phoff, 8) < 0 ||
	    target_read(pid, exe_map + 0x38, &phnum16, 2) < 0)
		return 0;
	e_phnum = phnum16;

	unsigned long dyn_vaddr = 0, load_min = (unsigned long)-1;
	for (uint64_t i = 0; i < e_phnum && i < 64; i++) {
		Elf64_Phdr ph;
		if (target_read(pid, exe_map + e_phoff + i * sizeof(ph),
				 &ph, sizeof(ph)) < 0)
			return 0;
		if (ph.p_type == PT_LOAD && ph.p_vaddr < load_min)
			load_min = ph.p_vaddr;
		if (ph.p_type == PT_DYNAMIC)
			dyn_vaddr = ph.p_vaddr;
	}
	if (dyn_vaddr == 0 || load_min == (unsigned long)-1) {
		fprintf(stderr, "No PT_DYNAMIC in target executable.\n");
		return 0;
	}
	unsigned long exe_bias = exe_map - load_min;
	unsigned long dyn_addr = exe_bias + dyn_vaddr;

	/* --- DT_DEBUG → r_debug → link_map head --- */
	uint64_t debug_ptr;
	if (find_dyn(pid, dyn_addr, DT_DEBUG, &debug_ptr) < 0 ||
	    debug_ptr == 0) {
		fprintf(stderr, "No DT_DEBUG (is the target started?).\n");
		return 0;
	}
	/* struct r_debug { int r_version; struct link_map *r_map; ... }:
	 * r_map sits at offset 8, NOT 0. */
	uint64_t r_map;
	if (read_u64(pid, debug_ptr + 8, &r_map) < 0 || r_map == 0) {
		fprintf(stderr, "Empty link_map (target not initialized?).\n");
		return 0;
	}

	/* --- walk link_map to libc --- */
	uint64_t libc_bias = 0;
	for (int hops = 0; hops < 128; hops++) {
		uint64_t l_addr, l_name, l_ld, l_next;
		if (read_u64(pid, r_map, &l_addr) < 0 ||
		    read_u64(pid, r_map + 8, &l_name) < 0 ||
		    read_u64(pid, r_map + 16, &l_ld) < 0 ||
		    read_u64(pid, r_map + 24, &l_next) < 0)
			return 0;
		if (l_name) {
			char name[256];
			if (target_read_str(pid, l_name, name, sizeof(name)) == 0 &&
			    strstr(name, "libc.so")) {
				libc_bias = l_addr;
				break;
			}
		}
		if (!l_next)
			break;
		r_map = l_next;
	}
	if (!libc_bias) {
		fprintf(stderr, "libc not found in target link_map.\n");
		return 0;
	}
	/* re-walk to fetch the libc entry's l_ld (kept simple + robust) */
	uint64_t libc_dyn = 0;
	{
		uint64_t head;
		if (read_u64(pid, debug_ptr + 8, &head) < 0)
			return 0;
		for (int hops = 0; hops < 128 && head; hops++) {
			uint64_t l_addr, l_name, l_ld, l_next;
			if (read_u64(pid, head, &l_addr) < 0 ||
			    read_u64(pid, head + 8, &l_name) < 0 ||
			    read_u64(pid, head + 16, &l_ld) < 0 ||
			    read_u64(pid, head + 24, &l_next) < 0)
				return 0;
			if (l_addr == libc_bias) {
				libc_dyn = l_ld;
				break;
			}
			head = l_next;
		}
	}
	if (!libc_dyn) {
		fprintf(stderr, "No PT_DYNAMIC for target libc.\n");
		return 0;
	}

	/* --- libc dynamic tags (link-time addrs → add bias) --- */
	uint64_t symtab = 0, strtab = 0, hash = 0, syment = 0;
	if (find_dyn(pid, libc_dyn, DT_SYMTAB, &symtab) < 0 ||
	    find_dyn(pid, libc_dyn, DT_STRTAB, &strtab) < 0 ||
	    find_dyn(pid, libc_dyn, DT_HASH, &hash) < 0 ||
	    find_dyn(pid, libc_dyn, DT_SYMENT, &syment) < 0 ||
	    !symtab || !strtab || !hash || !syment) {
		fprintf(stderr, "Target libc lacks SYSV hash tables.\n");
		return 0;
	}
	/*
	 * DT_* address tags: some loaders keep them as link-time values
	 * (add l_addr), others expose them pre-biased already. Instead of
	 * assuming, validate: a genuine SYSV hash table has bounded bucket
	 * counts and an in-range first bucket. Whichever candidate parses
	 * wins — for symtab, strtab and hash alike (one loader, one rule).
	 */
	{
		unsigned long try_hash[2] = { hash, hash + libc_bias };
		unsigned long try_sym[2] = { symtab, symtab + libc_bias };
		unsigned long try_str[2] = { strtab, strtab + libc_bias };
		int picked = -1;
		for (int c = 0; c < 2 && picked < 0; c++) {
			uint32_t nb = 0, nc = 0;
			if (target_read(pid, try_hash[c], &nb, 4) < 0 ||
			    target_read(pid, try_hash[c] + 4, &nc, 4) < 0)
				continue;
			if (nb == 0 || nb > (1U << 20) || nc > (1U << 24))
				continue;
			uint32_t b0 = 0;
			if (target_read(pid, try_hash[c] + 8, &b0, 4) < 0 ||
			    b0 >= nc)
				continue;
			picked = c;
			symtab = try_sym[c];
			strtab = try_str[c];
			hash = try_hash[c];
		}
		if (picked < 0) {
			fprintf(stderr,
				"Target libc hash tables unreadable either way.\n");
			return 0;
		}
	}

	uint32_t nbucket, nchain;
	if (target_read(pid, hash, &nbucket, 4) < 0 ||
	    target_read(pid, hash + 4, &nchain, 4) < 0 || !nbucket)
		return 0;

	unsigned long h = elf_hash(symname) % nbucket;
	uint32_t symidx;
	if (target_read(pid, hash + 8 + h * 4, &symidx, 4) < 0)
		return 0;
	for (uint32_t hops = 0; hops < nchain + 1 && symidx; hops++) {
		uint32_t name_off;
		unsigned long st_name_addr =
			symtab + (unsigned long)symidx * syment;
		if (target_read(pid, st_name_addr, &name_off, 4) < 0)
			return 0;
		char cand[128];
		if (target_read_str(pid, strtab + name_off, cand, sizeof(cand)) < 0)
			return 0;
		/* st_info is at +4, st_value at +8 (Elf64_Sym layout). */
		if (!strcmp(cand, symname)) {
			unsigned long st_value;
			uint8_t st_info;
			if (target_read(pid, st_name_addr + 8, &st_value, 8) < 0 ||
			    target_read(pid, st_name_addr + 4, &st_info, 1) < 0)
				return 0;
			if (st_value && ELF64_ST_TYPE(st_info) == STT_FUNC)
				return libc_bias + st_value;
			/* Name matches but not a function (e.g. object):
			 * keep searching instead of returning garbage. */
		}
		uint32_t next;
		if (target_read(pid, hash + 8 + (nbucket + symidx) * 4, &next, 4) < 0)
			return 0;
		symidx = next;
	}
	return 0;
}

static void poke_bytes(pid_t pid, unsigned long addr, const void *buf,
		       size_t len)
{
	const unsigned char *p = buf;
	size_t i = 0;
	while (i < len) {
		unsigned long word = 0;
		size_t chunk = len - i < sizeof(word) ? len - i : sizeof(word);
		if (chunk < sizeof(word)) {
			errno = 0;
			word = (unsigned long)ptrace(PTRACE_PEEKDATA, pid,
						     (void *)(addr + i), NULL);
			if (errno)
				die("PTRACE_PEEKDATA");
		}
		memcpy(&word, p + i, chunk);
		if (ptrace(PTRACE_POKEDATA, pid, (void *)(addr + i),
			   (void *)word) < 0)
			die("PTRACE_POKEDATA");
		i += chunk;
	}
}

/* Call func(arg0, arg1) in the target; result in *out_rax. Restores regs. */
static int remote_call2(pid_t pid, unsigned long func, unsigned long arg0,
			unsigned long arg1, unsigned long *out_rax)
{
	struct user_regs_struct saved, regs;
	if (ptrace(PTRACE_GETREGS, pid, NULL, &saved) < 0)
		die("GETREGS");
	regs = saved;

	/* 16-byte align the stack and plant a zero return address so the
	 * callee's `ret` traps back to us (SIGSEGV → wait stops it). */
	regs.rsp &= ~0xFUL;
	regs.rsp -= 8;
	unsigned long bad_ret = 0;
	poke_bytes(pid, regs.rsp, &bad_ret, sizeof(bad_ret));

	/* SysV call ABI for the first two args (libc wrapper functions). */
	regs.rip = func;
	regs.rdi = arg0;
	regs.rsi = arg1;
	regs.rax = 0;

	if (ptrace(PTRACE_SETREGS, pid, NULL, &regs) < 0)
		die("SETREGS");
	if (ptrace(PTRACE_CONT, pid, NULL, NULL) < 0)
		die("CONT");

	int status = wait_stopped(pid);
	if (status < 0)
		return -1;

	if (ptrace(PTRACE_GETREGS, pid, NULL, &regs) < 0)
		die("GETREGS after");
	if (out_rax)
		*out_rax = regs.rax;

	if (ptrace(PTRACE_SETREGS, pid, NULL, &saved) < 0)
		die("restore SETREGS");
	return 0;
}

static unsigned long remote_mmap(pid_t pid, size_t size,
				 unsigned long mmap_addr)
{
	struct user_regs_struct saved, regs;
	if (ptrace(PTRACE_GETREGS, pid, NULL, &saved) < 0)
		die("GETREGS");
	regs = saved;

	regs.rsp &= ~0xFUL;
	regs.rsp -= 8;
	unsigned long bad_ret = 0;
	poke_bytes(pid, regs.rsp, &bad_ret, sizeof(bad_ret));

	/* Calling the libc mmap WRAPPER: plain function ABI
	 * (rdi, rsi, rdx, rcx, r8, r9) — rcx, not r10. */
	regs.rip = mmap_addr;
	regs.rdi = 0;
	regs.rsi = size;
	regs.rdx = PROT_READ | PROT_WRITE;
	regs.rcx = MAP_PRIVATE | MAP_ANONYMOUS;
	regs.r8 = (unsigned long)-1;
	regs.r9 = 0;
	regs.rax = 0;

	if (ptrace(PTRACE_SETREGS, pid, NULL, &regs) < 0)
		die("SETREGS mmap");
	if (ptrace(PTRACE_CONT, pid, NULL, NULL) < 0)
		die("CONT mmap");
	if (wait_stopped(pid) < 0)
		return 0;

	if (ptrace(PTRACE_GETREGS, pid, NULL, &regs) < 0)
		die("GETREGS mmap");
	unsigned long mapped = regs.rax;

	if (ptrace(PTRACE_SETREGS, pid, NULL, &saved) < 0)
		die("restore after mmap");

	if (mapped == (unsigned long)-1 || mapped == 0) {
		fprintf(stderr, "Could not allocate memory in target process.\n");
		return 0;
	}
	return mapped;
}

/* Read the target's dlerror() string (0-arg call via remote_call2). */
static void report_target_dlerror(pid_t pid, unsigned long dlerror_addr)
{
	unsigned long msg_ptr = 0;
	if (remote_call2(pid, dlerror_addr, 0, 0, &msg_ptr) < 0 || !msg_ptr) {
		fprintf(stderr, "dlopen failed (no further detail).\n");
		return;
	}
	char msg[512];
	if (target_read_str(pid, msg_ptr, msg, sizeof(msg)) < 0) {
		fprintf(stderr, "dlopen failed (detail unreadable).\n");
		return;
	}
	fprintf(stderr, "Target dlerror: %s\n", msg);
}

int main(int argc, char **argv)
{
	/* No ptrace needed: resolve symbols in OURSELVES via the same
	 * link_map walk, and compare against dlsym(). Any mismatch means
	 * the parser is broken — catch it here, not in the game. */
	if (argc == 2 && !strcmp(argv[1], "--self-test")) {
		/* __libc_dlopen_mode is intentionally NOT tested: recent
		 * glibc no longer exports it dynamically (main() falls back
		 * to plain "dlopen", which is tested here). */
		const char *syms[] = { "mmap", "dlopen", "dlerror", NULL };
		int fails = 0;
		for (int i = 0; syms[i]; i++) {
			unsigned long got =
				resolve_target_libc_sym(getpid(), syms[i]);
			/* libc is in our own global scope: ground truth. */
			void *ref = dlsym(RTLD_DEFAULT, syms[i]);
			printf("%-20s walk=0x%lx dlsym=%p %s\n", syms[i], got,
			       ref, (got && got == (unsigned long)ref)
					 ? "MATCH"
					 : "MISMATCH");
			if (!got || got != (unsigned long)ref)
				fails++;
		}
		if (fails) {
			fprintf(stderr, "self-test FAILED\n");
			return 1;
		}
		printf("self-test OK\n");
		return 0;
	}

	if (argc != 3) {
		fprintf(stderr, "usage: %s <pid> /absolute/path/to.so\n",
			argv[0]);
		return 2;
	}

	errno = 0;
	long pid_l = strtol(argv[1], NULL, 10);
	if (errno || pid_l <= 0 || pid_l > INT_MAX) {
		fprintf(stderr, "Invalid process ID.\n");
		return 2;
	}
	pid_t pid = (pid_t)pid_l;

	char so_path[PATH_MAX];
	if (argv[2][0] != '/') {
		fprintf(stderr, "Library path must be absolute.\n");
		return 2;
	}
	if (!realpath(argv[2], so_path))
		die("realpath");
	if (access(so_path, R_OK) != 0)
		die("access .so");

	size_t path_len = strlen(so_path) + 1;

	if (ptrace(PTRACE_ATTACH, pid, NULL, NULL) < 0)
		die("PTRACE_ATTACH (need same uid? yama ptrace_scope?)");
	if (wait_stopped(pid) < 0)
		return 1;

	unsigned long mmap_addr =
		resolve_target_libc_sym(pid, "mmap");
	unsigned long dlopen_addr =
		resolve_target_libc_sym(pid, "__libc_dlopen_mode");
	if (!dlopen_addr)
		dlopen_addr = resolve_target_libc_sym(pid, "dlopen");
	unsigned long dlerror_addr =
		resolve_target_libc_sym(pid, "dlerror");
	if (!mmap_addr || !dlopen_addr) {
		fprintf(stderr, "Symbol resolution in target failed.\n");
		ptrace(PTRACE_DETACH, pid, NULL, NULL);
		return 1;
	}

	unsigned long remote_buf =
		remote_mmap(pid, (path_len + 0xFFF) & ~0xFFFUL, mmap_addr);
	if (!remote_buf) {
		ptrace(PTRACE_DETACH, pid, NULL, NULL);
		return 1;
	}
	poke_bytes(pid, remote_buf, so_path, path_len);

	unsigned long handle = 0;
	if (remote_call2(pid, dlopen_addr, remote_buf, DL_MODE, &handle) < 0) {
		ptrace(PTRACE_DETACH, pid, NULL, NULL);
		return 1;
	}

	if (ptrace(PTRACE_DETACH, pid, NULL, NULL) < 0)
		die("PTRACE_DETACH");

	if (!handle) {
		if (dlerror_addr) {
			/* Re-attach briefly to read the error string. */
			if (ptrace(PTRACE_ATTACH, pid, NULL, NULL) == 0 &&
			    wait_stopped(pid) >= 0) {
				report_target_dlerror(pid, dlerror_addr);
				ptrace(PTRACE_DETACH, pid, NULL, NULL);
			} else {
				fprintf(stderr,
					"dlopen failed (could not re-attach).\n");
			}
		} else {
			fprintf(stderr,
				"Could not load the library (check path and dependencies).\n");
		}
		return 1;
	}

	printf("Injected into process %d.\n", (int)pid);
	return 0;
}
