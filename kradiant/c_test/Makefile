CC = gcc
CFLAGS = -Wall -Wextra -I..
LDFLAGS = -L../target/release -lkradiant

all: test

test: main.c
	$(CC) $(CFLAGS) main.c $(LDFLAGS) -o test_runner

run: test
	LD_LIBRARY_PATH=../target/release ./test_runner

clean:
	rm -f test_runner
