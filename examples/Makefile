SRC = $(wildcard *.c)
OBJ = $(SRC:.c=.o)
TARGET = testlibos

CFLAGS += -I../include

all: $(TARGET)

$(TARGET): $(OBJ)
	$(CC) $(CFLAGS) -o $@ $^

%.o: %.c
	$(CC) $(CFLAGS) -c $< -o $@

clean:
	rm -f $(OBJ) $(TARGET)

.PHONY: all clean