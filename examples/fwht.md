---
title: Fast Walsh-Hadamard Transform in Competitive Programming
tags: [乱搞, OI, English]
date: 2020-08-07 04:56:28
update: 2020-08-07 04:56:28
---

This should be the very first English post I write on my blog and I expect there to be some minor errors. This is a popular technique in the Chinese competitive programming community but there doesn't seem to be a lot of documentation about its application in the English CP community. The posts I found on Codeforces doesn't seem to be very clear to me...

# Prerequisites

* A decent proficiency in competitive programming.
* A basic understanding of the Cooley-Tukey FFT and its application in competitive programming.
* A decent understanding of bitwise operations.

# Why do we need FWHT?

Recall what we would do if we are to quickly calculate the following convolution of two sequences $a$ and $b$, each of length $n$:
$$
\{a\circledast b\}_i=\sum_{j+k=i}a_{j}b_k \Bigg/ \{a\circledast b\}_i=\sum_{j=0}^ia_{j}b_{i-j}
$$
We use FFT, which applies the following transformation to the input sequence:
$$
\mathcal{F}\{a\}_i = \sum_{j=0}^n a_j\omega_{n}^{ij}
$$
Since the calculation of this transformation (and its inverse) can be done in a divide-and-conquer manner in $\mathcal O(n\log n)$ and the element wise product of the transformation is equivalent to the convolution on the original series, we are able to calculate the convolution in $\mathcal O(n\log n)$.

Now we try to generalize our findings to a more general case:
$$
\{a\circledast b\}_i=\sum_{j \star k=i}a_jb_k
$$
where $\star$ is some binary operation. The convolution we see at the beginning is a special case where $\star = +$.

FWHT is an algorithm that borrows similar notions from FFT and is able to compute the convolution in $\mathcal O(n \log n)$ time for $\star =\vee,\wedge,\oplus$ (bitwise OR, bitwise AND, and bitwise XOR). Why do the convolutions of these bitwise operations matter? Observe that binary representation is a way of encoding sets and these three operations correspond to set union, set intersection and set symmetric difference respectively, therefore, FWHT can be used to accelerate set-based DPs.

# Bitwise OR convolution

Let's start with the convolution with respect to bitwise OR:
$$
\{a\circledast b\}_i = \sum_{j\vee k=i}a_jb_k
$$
We start by exploiting an interesting property of bitwise OR:
$$
x \vee z = z,y\vee z = z \Leftrightarrow (x\vee y)\vee z=z
$$
or its clearer equivalent in set-based notations:
$$
X\subseteq Z,Y\subseteq Z \Leftrightarrow (X\cup Y)\subseteq Z
$$
**Claim:** The following transformation can turn OR convolutions to element-wise multiplications:
$$
\mathcal{FWHT}\{a\}_i=\sum_{j\vee i=i} a_j
$$
**Proof:** 
$$
\begin{aligned}
\mathcal{FWHT}\{a\}_i\cdot\mathcal{FWHT}\{b\}_i &= \left(\sum_{j\vee i=i}a_j\right) \left(\sum_{k\vee i=i}b_k\right) \\
&= \sum_{j\vee i=i}\sum_{k\vee i=i}a_jb_k \\
&= \sum_{(j\vee k)\vee i=i} a_jb_k \\
&= \sum_{l\vee i = i}\sum_{j \vee k = l} a_jb_k\\
&= \mathcal{FWHT}\{a\circledast b\}_i
\end{aligned}
$$
Then how are we able to compute $\mathcal{FWHT}\{a\}$ quickly? A trivial implementation still takes $\mathcal O(n^2)$ time.

Recall what we did in FFT: we divide $a$ into two subsequences based on parity of indices, a.k.a, the last bit of indices. We did this because the root of unity has such amazing property as $\omega_n^k=\omega_{n/2}^{k/2}$. We could do that here as well, but a limitation of dividing based on the last bit is that the order of elements changes in the process, so an efficient in-place implementation has to do a pre-shuffle to cope with that. Since OR is a bitwise operation, which bit based on which we divide doesn't really matter much. Why not simply **divide based on the first, or the most significant bit**, such that the order of elements is preserved in the process? Dividing based on the highest bit of indices, simply put, is to split $a$ into the first half, $a^0$, and the second half, $a^1$, in their natural order.

Here I introduce a notation, $1|a$ or $0|a$. In the context where the length of the sequence is $n$ (and $n$ is a power of $2$), $1|a=\frac{n}{2}+a$ where $0\le a<n/2$, and $0|a$ is just $a$. In other words, $1|a$ has $1$ as the highest bit and $0|a$ has $0$ as the highest bit.

(Note using this notation, $a^0_i = a_{0|i}$ and $a^1_i = a_{1|i}$)

To make our writing clearer, denote
$$
\begin{aligned}
\mathcal{FWHT}\{a\} &= A \\
\mathcal{FWHT}\left\{a^0\right\} &= A^0 \\
\mathcal{FWHT}\left\{a^1\right\} &= A^1 \\
\end{aligned}
$$
We want to express each element of $A$ as a combination of some element in $A^0$ and $A^1$. 

We first look at the first half of $A$. Using the notation I defined above, these elements can be expressed as $A_{0|i}$.
$$
\begin{aligned}
A_{0|i}&=\sum_{j\vee (0|i)=0|i}a_j \\
&= \sum_{(0|j)\vee (0|i)=0|i}a^0_j + \sum_{(1|j)\vee (0|i)=0|i}a^1_j \\
\end{aligned}
$$
We know that the highest bit of $(1|j)\vee (0|i)$ should always be $1$, so the condition in the second summation is never satisfied, and we can simply throw the second term away. And $(0|j)\vee (0|i)=0|i$ simplifies to $j\vee i =i$, so we get, by definition of $A^0$:
$$
A_{0|i} = A^0_i
$$
What about the second half, $A_{1|i}$?
$$
\begin{aligned}
A_{1|i}&=\sum_{j\vee (1|i)=1|i}a_j \\
&= \sum_{(0|j)\vee (1|i)=1|i}a^0_j + \sum_{(1|j)\vee (1|i)=1|i}a^1_j \\
&= A_i^0+A_i^1
\end{aligned}
$$
Together we get:
$$
A=\left(A^0, A^0+A^1\right)
$$
with the trivial recursion boundary $A_0=a_0$ when $n=0$.

(here I use the tuple notation to denote concatenation, and $+$ to denote element-wise addition).

This is something we can write an in-place implementation for with ease:

```cpp
void fwht_or(int n, int *a, int *A) {
    copy(a, a + n, A);
    for (int s = 2, h = 1; s <= n; s <<= 1, h <<= 1)
        for (int l = 0; l < n; l += s)
            for (int i = 0; i < h; i++)
                A[l + h + i] += A[l + i]
}
```

Its time complexity is obviously $\mathcal O(n\log n)$ with a really small constant factor.

Its reverse transform turns out to be simple as well, suppose we know $A$ and let
$$
A=(A',A'')
$$
(Assuming $n$ is a power of $2$ and $A'$ and $A''$ each have length $n/2$)

Then we can recover $A^0$ and $A^1$:
$$
\begin{cases}
	A^0=A'\\
	A^1=A''-A'
\end{cases}
$$
Implementation:

```cpp
void ifwht_or(int n, int *a, int *A) { 
    std::copy(A, A + n, a);
    // If n is guaranteed to be a power of 2 then we don't need n_ and the min(...) in the inner loop.
    int n_ = 1; while (n_ < n) n_ <<= 1; 
    // n_ = 1 << (32 - __builtin_clz(n - 1));
    for (int s = n_, h = n_ / 2; h; s >>= 1, h >>= 1)
        for (int l = 0; l < n; l += s)
            for (int i = 0; i < std::min(i, n - l - h); i++)
                a[l + h + i] -= a[l + i]
}
```

And an amazing thing about this, which I haven't quite figured out why, is that the order of the outermost loop above can be reversed and both functions can be merged into one:

```cpp
void fwht_or(int n, int *a, int *A, int dir = 1) {
    std::copy(a, a + n, A);
    for (int s = 2, h = 1; s <= n; s <<= 1, h <<= 1)
        for (int l = 0; l < n; l += s)
            for (int i = 0; i < h; i++)
                A[l + h + i] += dir * A[l + i]
}
```

(Fast bitwise OR / set union convolution is sometimes aliased "Fast Mobius Transform" in Chinese CP community. Both are essentially the same.)

# Bitwise AND convolution

The bitwise AND convolution
$$
\{a\circledast b\}_i = \sum_{j\wedge k=i}a_jb_k
$$
can be accelerated in a similar way. 

(Actually, by de Morgan's Law we can always reduce an AND convolution to an OR convolution)

Note that AND also has this interesting property:
$$
x \wedge z = z,y\wedge z = z \Leftrightarrow (x\wedge y)\wedge z=z
$$
or in set notations:
$$
Z\subseteq X,Z\subseteq Y \Leftrightarrow Z\subseteq(X\cap Y)
$$
Thus, we can prove in a way similar to what we did in OR convolution that the transform
$$
\mathcal{FWHT}\{a\}_i=\sum_{j\wedge i=i} a_j
$$
can turn convolutions to element-wise multiplications.

We still adopt the same divide-and-conquer approach and continue to use the notations $a, a^0,a^1,A,A^0,A^1$.

Consider the first half of $A$, which can be expressed as $A_{0|i}$:
$$
\begin{aligned}
A_{0|i}&=\sum_{j\wedge (0|i)=0|i}a_j \\
&= \sum_{(0|j)\wedge (0|i)=0|i}a^0_j + \sum_{(1|j)\wedge (0|i)=0|i}a^1_j \\
\end{aligned}
$$
And by the properties of AND, both $(0|j)\wedge (0|i)=0|i$ and $(1|j)\wedge (0|i)=0|i$ simplify to $j\wedge i=i$. So by definition we get
$$
A_{0|i}=A^0_i+A_i^1
$$
Then consider the other half of $A$:
$$
\begin{aligned}
A_{1|i}&=\sum_{j\wedge (1|i)=1|i}a_j \\
&= \sum_{(0|j)\wedge (1|i)=1|i}a^0_j + \sum_{(1|j)\wedge (1|i)=1|i}a^1_j \\
&= A_i^1
\end{aligned}
$$
Together we have:
$$
A=\left(A^0+A^1,A^1\right)
$$
with the trivial recursion boundary $A_0=a_0$ when $n=0$.

This gives an efficient implementation very similar to `fwht_or` above:

```cpp
void fwht_and(int n, int *a, int *A) {
    std::copy(a, a + n, A);
    for (int s = 2, h = 1; s <= n; s <<= 1, h <<= 1)
        for (int l = 0; l < n; l += s)
            for (int i = 0; i < h; i++)
                A[l + i] += A[l + h + i]
}
```

The inverse transform is simple as well. Let $A=(A', A'')$, then
$$
\begin{cases}
A^0=A'-A'' \\
A^1=A''
\end{cases}
$$
The code:

```cpp
void ifwht_and(int n, int *a, int *A) { 
    std::copy(A, A + n, a);
    // If n is guaranteed to be a power of 2 then we don't need n_ and the min(...) in the inner loop.
    int n_ = 1; while (n_ < n) n_ <<= 1; 
    for (int s = n_, h = n_ / 2; h; s >>= 1, h >>= 1)
        for (int l = 0; l < n; l += s)
            for (int i = 0; i < std::min(i, n - l - h); i++)
                a[l + i] -= a[l + h + i]
}
```

The order of the outermost loop can be reversed and we can also merge the functions above together. 

# Bitwise XOR convolution

The XOR operation does **not** have such nice property as 
$$
x\oplus z=z, y\oplus z=z\Leftrightarrow (x\oplus y)\oplus z=z
$$
So accelerating the convolution
$$
\{a\circledast b\}_i = \sum_{j\oplus k=i}a_jb_k
$$
is not as straightforward as we did above.

We first introduce an auxiliary operation, define $x \otimes y= \operatorname{popcount}(x\wedge y) \bmod 2$, where $\operatorname{popcount}(x)$ denotes the number of $1$s in the binary representation of $x$.

**Claim:** The transformation below turns convolutions to element-wise multiplications:
$$
\mathcal{FWHT}\{a\}_i=\sum_{j\otimes i=0} a_j - \sum_{j\otimes i=1} a_j
$$
**Proof: **
$$
\begin{aligned}
\mathcal{FWHT}\{a\}_i\cdot\mathcal{FWHT}\{b\}_i &= \left(\sum_{j\otimes i=0} a_j - \sum_{j\otimes i=1} a_j\right) \left(\sum_{k\otimes i=0} b_k - \sum_{k\otimes i=1} b_k\right) \\
&= \sum_{j\otimes i=0}\sum_{k\otimes i=0}a_jb_k +\sum_{j\otimes i=1}\sum_{k\otimes i=1}a_jb_k \\
&- \sum_{j\otimes i=1}\sum_{k\otimes i=0}a_jb_k - \sum_{j\otimes i=0}\sum_{k\otimes i=1}a_jb_k
\end{aligned}
$$
How to simplify those terms?

Observe that by the definition of XOR we have
$$
\operatorname{popcount}(x\oplus y) = \operatorname{popcount}(x)+\operatorname{popcount}(y)-2\operatorname{popcount}(x\wedge y)
$$
So if we apply modulo $2$ on both sides,
$$
\operatorname{popcount}(x\oplus y) \equiv \operatorname{popcount}(x)+\operatorname{popcount}(y) \pmod 2
$$
Plug in $x=j\wedge i,y=k\wedge i$ and we get
$$
\operatorname{popcount}((j\wedge i)\oplus (k\wedge i)) \equiv \operatorname{popcount}(j\wedge i)+\operatorname{popcount}(k\wedge i) \pmod 2
$$
We are almost there. Apply the identity below, whose proof I simply omit here,
$$
(j\wedge i)\oplus (k\wedge i)=(j\oplus k)\wedge i
$$
(This is something good about bitwise operations: if you cannot prove an identity the smart way you can always fall back on the dumb method -- making a truth table)

We finally get
$$
(j \oplus k)\otimes i \equiv j\otimes i+k\otimes i \pmod 2
$$
(We are actually quite familiar with this if we remove the circles outside $+$s and $\times$s)

With this conclusion we can simplify the four terms above:
$$
\begin{aligned}
\mathcal{FWHT}\{a\}_i\cdot\mathcal{FWHT}\{b\}_i &= \cdots \\
&= \sum_{(j\oplus k)\otimes i=0}a_jb_k - \sum_{(j\oplus k)\otimes i=1}a_jb_k \\
&=  \mathcal{FWHT}\{a\circledast b\}_i
\end{aligned}
$$
which completes the proof.

We then explore how to compute $\mathcal{FWHT}\{a\}$ efficiently. Divide and conquer is still our friend, and dividing $a$ based on the highest bit works here so we continue to use those notations.

Consider the first half of $A$...
$$
\begin{aligned}
A_{0|i} &= \sum_{j\otimes (0|i)=0} a_j - \sum_{j\otimes (0|i)=1} a_j \\
&= \sum_{(0|j)\otimes (0|i)=0} a_j^0+  \sum_{(1|j)\otimes (0|i)=0}a_j^1 -  \sum_{(0|j)\otimes (0|i)=1} a_j^0-  \sum_{(1|j)\otimes (0|i)=1}a_j^1 \\
&= \sum_{j\otimes i=0} a_j^0+  \sum_{j\otimes i=0}a_j^1 -  \sum_{j\otimes i=1} a_j^0-  \sum_{j\otimes i=1}a_j^1 \\
&= A_i^0
+A_i^1\end{aligned}
$$
and the other half:
$$
\begin{aligned}
A_{1|i} &= \sum_{j\otimes (1|i)=0} a_j - \sum_{j\otimes (1|i)=1} a_j \\
&= \sum_{(0|j)\otimes (1|i)=0} a_j^0+  \sum_{(1|j)\otimes (1|i)=0}a_j^1 -  \sum_{(0|j)\otimes (1|i)=1} a_j^0-  \sum_{(1|j)\otimes (1|i)=1}a_j^1 \\
&= \sum_{j\otimes i=0} a_j^0+  \sum_{j\otimes i=1}a_j^1 -  \sum_{j\otimes i=1} a_j^0-  \sum_{j\otimes i=0}a_j^1 \\
&= A_i^0 - A_i^1
\end{aligned}
$$
So together we get
$$
A=\left(A^0+A^1,A^0-A^1\right)
$$
and the inverse transform
$$
A=(A',A'') \Rightarrow \begin{cases}
\displaystyle A^0=\frac{A'+A''}{2} \\
\displaystyle A^1=\frac{A'-A''}{2}
\end{cases}
$$
The code for both transforms are a bit longer than those for OR and AND, but not by too much:

```cpp
void fwht_xor(int n, int *a, int *A) {
    std::copy(a, a + n, A);
    for (int s = 2, h = 1; s <= n; s <<= 1, h <<= 1) {
        for (int l = 0; l < n; l += s) {
            for (int i = 0; i < h; i++) {
                int t = A[l + h + i];
                A[l + h + i] = A[l + i] - t;
                A[l + i] += t;
            }
        }
    }
}
void ifwht_xor(int n, int *a, int *A) {
    std::copy(A, A + n, a);
    // If n is guaranteed to be a power of 2 then we don't need n_ and the min(...) in the inner loop.
    int n_ = 1; while (n_ < n) n_ <<= 1; 
    for (int s = 2, h = 1; s <= n; s <<= 1, h <<= 1) {
        for (int l = 0; l < n; l += s) {
            for (int i = 0; i < std::min(i, n - l - h); i++) {
                int t = a[l + h + i];
                a[l + h + i] = (a[l + i] - t) / 2;
                a[l + i] = (a[l + i] + t) / 2;
            }
        }
    }
}
```

They can be merged as well:

```cpp
void fwht_xor(int n, int *a, int *A, bool inv = false) {
    std::copy(a, a + n, A);
    for (int s = 2, h = 1; s <= n; s <<= 1, h <<= 1) {
        for (int l = 0; l < n; l += s) {
            for (int i = 0; i < h; i++) {
                int t = A[l + h + i];
                A[l + h + i] = A[l + i] - t;
                A[l + i] += t;
                if (inv) A[l + h + i] /= 2, A[l + i] /= 2;
            }
        }
    }
}
```

**This code above is what Wikipedia refers to as the authentic Fast Walsh-Hadamard Transform**.

# Some sidenotes

Note that though FFT and FWHT shares the same idea of divide and conquer, FWHT does not require $n$ to be a power of $2$ whereas FFT does. (Well actually neither of them "require" $n$ to be a power of $2$, but to apply FFT when $n$ is not a power of $2$ you either need to pad with $0$s or you have to make your implementation really complicated).

Also, I just came to know that if we express WHT in the language of matrices and vectors, the matrix is called a Hadamard Matrix.

Another fact that I didn't quite understand is why the order of the inverse FWHT can be reversed.

For instance, when $n=8$, after fully dividing the sequence into individual elements, we first merge $(0,1),(2,3),(4,5),(6,7)$, then we merge $(0,1,2,3),(4,5,6,7)$ and finally $(0,1,2,3,4,5,6,7)$. Naturally when we do the inverse transform we have to start with $(0,1,2,3,4,5,6,7)$, recover $(0,1,2,3),(4,5,6,7)$, then recover $(0,1),(2,3),(4,5),(6,7)$ and then recover the individual elements. But the popular implementation seems to suggest that the inverse transformation algorithm works in another direction as well. I am now puzzled why this is true and currently I'm just taking this for granted. Perhaps I derived the inversion in a different way than others did? If you have a simple explanation please leave a comment :)