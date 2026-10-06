#include <stdio.h>

int main(void) {
    long long left;
    long long right;
    if (scanf("%lld %lld", &left, &right) != 2) {
        return 1;
    }
    printf("%lld\n", left + right);
    return 0;
}
