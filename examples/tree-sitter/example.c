#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define MAX_ITEMS 64

typedef struct Node {
    int value;
    struct Node *next;
} Node;

typedef struct {
    Node *head;
    int   size;
} LinkedList;

LinkedList *list_new(void) {
    LinkedList *l = malloc(sizeof(LinkedList));
    if (!l) return NULL;
    l->head = NULL;
    l->size = 0;
    return l;
}

int list_push(LinkedList *l, int value) {
    Node *node = malloc(sizeof(Node));
    if (!node) return -1;
    node->value = value;
    node->next  = l->head;
    l->head     = node;
    l->size++;
    return 0;
}

int list_pop(LinkedList *l, int *out) {
    if (!l->head) return -1;
    Node *node = l->head;
    *out   = node->value;
    l->head = node->next;
    l->size--;
    free(node);
    return 0;
}

void list_free(LinkedList *l) {
    Node *cur = l->head;
    while (cur) {
        Node *next = cur->next;
        free(cur);
        cur = next;
    }
    free(l);
}

/* Binary search on a sorted array */
int bsearch_int(const int *arr, int len, int target) {
    int lo = 0, hi = len - 1;
    while (lo <= hi) {
        int mid = lo + (hi - lo) / 2;
        if (arr[mid] == target) return mid;
        if (arr[mid] < target)  lo = mid + 1;
        else                    hi = mid - 1;
    }
    return -1;
}

int main(void) {
    LinkedList *list = list_new();
    for (int i = 0; i < 5; i++) {
        list_push(list, i * 10);
    }

    int val;
    while (list_pop(list, &val) == 0) {
        printf("%d\n", val);
    }
    list_free(list);

    const int sorted[] = {1, 3, 5, 7, 9, 11, 13};
    int idx = bsearch_int(sorted, 7, 7);
    printf("found 7 at index %d\n", idx);

    return 0;
}
