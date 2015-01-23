#include "benchmark.h"
char tigerCorpus[] = "tiger2";

class TigerFixture : public CorpusFixture<true, tigerCorpus>
{
public:
  virtual ~TigerFixture() {}
};
class TigerFallbackFixture : public CorpusFixture<false, tigerCorpus>
{
public:
  virtual ~TigerFallbackFixture() {}
};


BASELINE_F(Tiger_Cat, Fallback, TigerFallbackFixture, 5, 1)
{
  AnnotationNameSearch search(db, "cat");
  counter=0;
  while(search.hasNext())
  {
    search.next();
    counter++;
  }
  assert(counter == 373436u);
}

BENCHMARK_F(Tiger_Cat, Optimized, TigerFixture, 5, 1)
{
  AnnotationNameSearch search(db, "cat");
  counter=0;
  while(search.hasNext())
  {
    search.next();
    counter++;
  }
  assert(counter == 373436u);
}

// cat="S" & tok="Bilharziose" & #1 >* #2
BASELINE_F(Tiger_BilharzioseSentence,  Fallback, TigerFallbackFixture, 5, 1)
{
  Query q(db);
  auto n1 = q.addNode(std::make_shared<AnnotationNameSearch>(db, "tiger", "cat", "S"));
  auto n2 = q.addNode(std::make_shared<AnnotationNameSearch>(db, annis_ns, annis_tok, "Bilharziose"));

  q.addOperator(std::make_shared<Dominance>(db, "", "", 1, uintmax), n1, n2);

  while(q.hasNext())
  {
    q.next();
    counter++;
  }

  assert(counter == 21u);
}
BENCHMARK_F(Tiger_BilharzioseSentence, Optimized, TigerFixture, 5, 1)
{
  Query q(db);
  auto n1 = q.addNode(std::make_shared<AnnotationNameSearch>(db, "tiger", "cat", "S"));
  auto n2 = q.addNode(std::make_shared<AnnotationNameSearch>(db, annis_ns, annis_tok, "Bilharziose"));

  q.addOperator(std::make_shared<Dominance>(db, "", "", 1, uintmax), n1, n2);

  while(q.hasNext())
  {
    q.next();
    counter++;
  }

  assert(counter == 21u);
}

// pos="NN" .2,10 pos="ART" . pos="NN"
BASELINE_F(Tiger_NNPreARTPreNN, Fallback, TigerFallbackFixture, 5, 1) {

  Query q(db);
  q.addNode(std::make_shared<AnnotationNameSearch>(db, "tiger", "pos", "NN"));
  q.addNode(std::make_shared<AnnotationNameSearch>(db, "tiger", "pos", "ART"));
  q.addNode(std::make_shared<AnnotationNameSearch>(db, "tiger", "pos", "NN"));

  q.addOperator(std::make_shared<Precedence>(db, 2,10), 0, 1);
  q.addOperator(std::make_shared<Precedence>(db), 1, 2);
  while(q.hasNext())
  {
    q.next();
    counter++;
  }
  assert(counter == 114042u);
}

BENCHMARK_F(Tiger_NNPreARTPreNN, Optimized, TigerFixture, 5, 1) {

  Query q(db);
  q.addNode(std::make_shared<AnnotationNameSearch>(db, "tiger", "pos", "NN"));
  q.addNode(std::make_shared<AnnotationNameSearch>(db, "tiger", "pos", "ART"));
  q.addNode(std::make_shared<AnnotationNameSearch>(db, "tiger", "pos", "NN"));

  q.addOperator(std::make_shared<Precedence>(db, 2,10), 0, 1);
  q.addOperator(std::make_shared<Precedence>(db), 1, 2);
  while(q.hasNext())
  {
    q.next();
    counter++;
  }
  assert(counter == 114042u);
}

// cat=/(.P)/ >* /A.*/
BASELINE_F(Tiger_RegexDom, Fallback, TigerFallbackFixture , 5, 1) {

  Query q(db);
  auto n1 = q.addNode(std::make_shared<RegexAnnoSearch>(db,
                                                        "cat",".P"));
  auto n2 = q.addNode(std::make_shared<RegexAnnoSearch>(db,
                                                        annis_ns, annis_tok,
                                                       "A.*"));

  q.addOperator(std::make_shared<Dominance>(db, "", "", 1, uintmax), n1, n2);

  while(q.hasNext())
  {
    std::vector<Match> m = q.next();
    counter++;
  }
  assert(counter == 36294u);
}

BENCHMARK_F(Tiger_RegexDom, Optimized, TigerFixture , 5, 1) {

  Query q(db);
  auto n1 = q.addNode(std::make_shared<RegexAnnoSearch>(db,
                                                        "cat",".P"));
  auto n2 = q.addNode(std::make_shared<RegexAnnoSearch>(db,
                                                        annis_ns, annis_tok,
                                                       "A.*"));

  q.addOperator(std::make_shared<Dominance>(db, "", "", 1, uintmax), n1, n2);

  while(q.hasNext())
  {
    std::vector<Match> m = q.next();
    counter++;
  }
  assert(counter == 36294u);
}

